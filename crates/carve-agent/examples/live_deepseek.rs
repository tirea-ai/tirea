//! Live test with DeepSeek API.
//!
//! Run with:
//! ```bash
//! export DEEPSEEK_API_KEY=<your-key>
//! cargo run --example live_deepseek
//! ```

use async_trait::async_trait;
use carve_agent::{
    run_loop, run_loop_stream, tool_map_from_arc, AgentConfig, AgentEvent, AgentLoopError, Context,
    FileStorage, Message, RunContext, Session, Storage, Tool, ToolDescriptor, ToolError,
    ToolResult,
};
use carve_state_derive::State;
use futures::StreamExt;
use genai::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// A simple calculator tool for testing.
struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "calculator",
            "Calculator",
            "Perform arithmetic calculations",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The arithmetic expression to evaluate (e.g., '2 + 3 * 4')"
                }
            },
            "required": ["expression"]
        }))
    }

    async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let expr = args["expression"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'expression'".to_string()))?;

        // Simple evaluation (for demo - real impl would use a proper parser)
        let result = eval_simple_expr(expr);

        Ok(ToolResult::success(
            "calculator",
            json!({ "result": result, "expression": expr }),
        ))
    }
}

/// Simple arithmetic evaluator (handles basic +, -, *, /)
fn eval_simple_expr(expr: &str) -> f64 {
    // Very basic parser - just for demo
    let expr = expr.replace(" ", "");

    // Try to parse as a simple number first
    if let Ok(n) = expr.parse::<f64>() {
        return n;
    }

    // Handle simple binary operations
    for op in ['+', '-', '*', '/'] {
        if let Some(pos) = expr.rfind(op) {
            if pos > 0 {
                let left = &expr[..pos];
                let right = &expr[pos + 1..];
                let l = eval_simple_expr(left);
                let r = eval_simple_expr(right);
                return match op {
                    '+' => l + r,
                    '-' => l - r,
                    '*' => l * r,
                    '/' => {
                        if r != 0.0 {
                            l / r
                        } else {
                            f64::NAN
                        }
                    }
                    _ => 0.0,
                };
            }
        }
    }

    0.0
}

/// State for counter tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
struct CounterState {
    counter: i64,
}

/// A weather tool (for parallel testing with calculator).
struct WeatherTool;

#[async_trait]
impl Tool for WeatherTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "get_weather",
            "Get Weather",
            "Get current weather for a city",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "City name"
                }
            },
            "required": ["city"]
        }))
    }

    async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let city = args["city"].as_str().unwrap_or("Unknown");
        // Simulate weather data
        Ok(ToolResult::success(
            "get_weather",
            json!({
                "city": city,
                "temperature": 22,
                "condition": "sunny",
                "humidity": 45
            }),
        ))
    }
}

/// A tool that sometimes fails (for error recovery testing).
struct UnreliableTool {
    fail_count: AtomicU32,
    max_failures: u32,
}

impl UnreliableTool {
    fn new(max_failures: u32) -> Self {
        Self {
            fail_count: AtomicU32::new(0),
            max_failures,
        }
    }
}

#[async_trait]
impl Tool for UnreliableTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("unreliable_api", "Unreliable API", "An API that may fail")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The query to process"
                    }
                },
                "required": ["query"]
            }))
    }

    async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let query = args["query"].as_str().unwrap_or("unknown");
        let count = self.fail_count.fetch_add(1, Ordering::SeqCst);

        if count < self.max_failures {
            Err(ToolError::ExecutionFailed(format!(
                "Network timeout (attempt {}/{})",
                count + 1,
                self.max_failures + 1
            )))
        } else {
            Ok(ToolResult::success(
                "unreliable_api",
                json!({
                    "result": format!("Successfully processed: {}", query),
                    "attempts": count + 1
                }),
            ))
        }
    }
}

/// A tool that always requests more information (for max_rounds testing).
struct InfiniteLoopTool;

#[async_trait]
impl Tool for InfiniteLoopTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("check_status", "Check Status", "Check system status").with_parameters(
            json!({
                "type": "object",
                "properties": {
                    "component": {
                        "type": "string",
                        "description": "Component to check"
                    }
                },
                "required": ["component"]
            }),
        )
    }

    async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let component = args["component"].as_str().unwrap_or("unknown");
        // Always return "needs more checks" to trigger max_rounds
        Ok(ToolResult::success(
            "check_status",
            json!({
                "status": "partial",
                "component": component,
                "message": "Need to check more components. Please also check 'database' and 'cache'."
            }),
        ))
    }
}

/// A counter tool that modifies state.
struct CounterTool;

#[async_trait]
impl Tool for CounterTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("counter", "Counter", "Increment or set a counter value")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["increment", "decrement", "set", "get"],
                        "description": "The action to perform"
                    },
                    "value": {
                        "type": "integer",
                        "description": "The value for set action or increment amount"
                    }
                },
                "required": ["action"]
            }))
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'action'".to_string()))?;

        // Access counter from typed state
        let state = ctx.state::<CounterState>("");
        let current = state.counter().unwrap_or(0);

        let new_value = match action {
            "get" => {
                return Ok(ToolResult::success(
                    "counter",
                    json!({ "value": current, "action": "get" }),
                ));
            }
            "increment" => {
                let amount = args["value"].as_i64().unwrap_or(1);
                current + amount
            }
            "decrement" => {
                let amount = args["value"].as_i64().unwrap_or(1);
                current - amount
            }
            "set" => args["value"].as_i64().ok_or_else(|| {
                ToolError::InvalidArguments("Missing 'value' for set".to_string())
            })?,
            _ => {
                return Err(ToolError::InvalidArguments(format!(
                    "Unknown action: {}",
                    action
                )))
            }
        };

        // Update state
        state.set_counter(new_value);

        Ok(ToolResult::success(
            "counter",
            json!({ "previous": current, "current": new_value, "action": action }),
        ))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check for API key
    if std::env::var("DEEPSEEK_API_KEY").is_err() {
        eprintln!("Error: DEEPSEEK_API_KEY environment variable not set");
        eprintln!("Usage: export DEEPSEEK_API_KEY=<your-key> && cargo run --example live_deepseek");
        std::process::exit(1);
    }

    println!("=== DeepSeek Live Test ===\n");

    // Create client
    let client = Client::default();

    // Create tools
    let calc_tool: Arc<dyn Tool> = Arc::new(CalculatorTool);
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([calc_tool, counter_tool]);

    // Test 1: Simple conversation (no tools)
    println!("--- Test 1: Simple Conversation ---");
    test_simple_conversation(&client).await?;

    // Test 2: Tool calling
    println!("\n--- Test 2: Calculator Tool ---");
    test_calculator(&client, &tools).await?;

    // Test 3: Streaming
    println!("\n--- Test 3: Streaming Response ---");
    test_streaming(&client).await?;

    // Test 4: Tool with state
    println!("\n--- Test 4: Counter Tool with State ---");
    test_counter_with_state(&client).await?;

    // Test 5: Multi-run conversation
    println!("\n--- Test 5: Multi-run Conversation ---");
    test_multi_run(&client).await?;

    // Test 6: Multi-run with tools
    println!("\n--- Test 6: Multi-run with Tools ---");
    test_multi_run_with_tools(&client).await?;

    // Test 7: Session persistence and restore
    println!("\n--- Test 7: Session Persistence & Restore ---");
    test_session_persistence(&client).await?;

    // Test 8: Parallel tool calls
    println!("\n--- Test 8: Parallel Tool Calls ---");
    test_parallel_tool_calls(&client).await?;

    // Test 9: Max rounds limit
    println!("\n--- Test 9: Max Rounds Limit ---");
    test_max_rounds_limit(&client).await?;

    // Test 10: Tool failure recovery
    println!("\n--- Test 10: Tool Failure Recovery ---");
    test_tool_failure_recovery(&client).await?;

    // Test 11: Session snapshot continuation
    println!("\n--- Test 11: Session Snapshot Continuation ---");
    test_session_snapshot(&client).await?;

    // Test 12: State replay
    println!("\n--- Test 12: State Replay ---");
    test_state_replay(&client).await?;

    // Test 13: Long conversation performance
    println!("\n--- Test 13: Long Conversation Performance ---");
    test_long_conversation(&client).await?;

    println!("\n=== All tests completed! ===");
    Ok(())
}

async fn test_simple_conversation(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    let session =
        Session::new("test-simple").with_message(Message::user("What is 2 + 2? Reply briefly."));

    let (session, response) = run_loop(client, &config, session, &std::collections::HashMap::new())
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("User: What is 2 + 2? Reply briefly.");
    println!("Assistant: {}", response);
    println!("Messages in session: {}", session.message_count());

    Ok(())
}

async fn test_calculator(
    client: &Client,
    tools: &std::collections::HashMap<String, Arc<dyn Tool>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(3);

    let session = Session::new("test-calc")
        .with_message(Message::system("You are a helpful assistant. Use the calculator tool when asked to perform calculations."))
        .with_message(Message::user("Please calculate 15 * 7 + 23 using the calculator tool."));

    let (session, response) = run_loop(client, &config, session, tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("User: Please calculate 15 * 7 + 23 using the calculator tool.");
    println!("Assistant: {}", response);
    println!("Messages in session: {}", session.message_count());
    println!("Patches in session: {}", session.patch_count());

    // Show tool calls from messages
    for msg in &session.messages {
        if let Some(ref calls) = msg.tool_calls {
            for call in calls {
                println!("Tool called: {} with args: {}", call.name, call.arguments);
            }
        }
    }

    Ok(())
}

async fn test_streaming(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    let session = Session::new("test-stream")
        .with_message(Message::user("Count from 1 to 5, one number per line."));

    let tools = std::collections::HashMap::new();

    println!("User: Count from 1 to 5, one number per line.");
    print!("Assistant: ");

    let mut stream = run_loop_stream(
        client.clone(),
        config,
        session,
        tools,
        RunContext::default(),
    );

    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::TextDelta { delta } => {
                print!("{}", delta);
                std::io::Write::flush(&mut std::io::stdout())?;
            }
            AgentEvent::RunFinish { result, .. } => {
                let response = result
                    .as_ref()
                    .and_then(|v| v.get("response"))
                    .and_then(|r| r.as_str())
                    .unwrap_or_default();
                println!("\n[Stream completed: {} chars]", response.len());
            }
            AgentEvent::Error { message } => {
                println!("\n[Error: {}]", message);
            }
            _ => {}
        }
    }

    Ok(())
}

async fn test_counter_with_state(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(5);

    // Create session with initial state
    let session = Session::with_initial_state("test-counter", json!({ "counter": 0 }))
        .with_message(Message::system("You are a helpful assistant. Use the counter tool to manage a counter. Always use the tool when asked about the counter."))
        .with_message(Message::user("Please increment the counter by 5, then increment it by 3 more. Tell me the final value."));

    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    let (session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("User: Please increment the counter by 5, then increment it by 3 more. Tell me the final value.");
    println!("Assistant: {}", response);
    println!("Messages in session: {}", session.message_count());
    println!("Patches in session: {}", session.patch_count());

    // Rebuild state to see final counter value
    let final_state = session.rebuild_state()?;
    println!(
        "Final state: {}",
        serde_json::to_string_pretty(&final_state)?
    );

    Ok(())
}

async fn test_multi_run(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    // Run 1: Introduce a topic
    let mut session = Session::new("test-multi-run")
        .with_message(Message::system(
            "You are a helpful assistant. Keep your answers brief.",
        ))
        .with_message(Message::user("My name is Alice. Remember it."));

    let (new_session, response) =
        run_loop(client, &config, session, &std::collections::HashMap::new())
            .await
            .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Run 1 - User: My name is Alice. Remember it.");
    println!("Run 1 - Assistant: {}", response);

    // Run 2: Test if context is remembered
    session = session.with_message(Message::user("What is my name?"));

    let (new_session, response) =
        run_loop(client, &config, session, &std::collections::HashMap::new())
            .await
            .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Run 2 - User: What is my name?");
    println!("Run 2 - Assistant: {}", response);

    // Run 3: Another follow-up
    session = session.with_message(Message::user("Say my name backwards."));

    let (session, response) = run_loop(client, &config, session, &std::collections::HashMap::new())
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("Run 3 - User: Say my name backwards.");
    println!("Run 3 - Assistant: {}", response);
    println!("Total messages in session: {}", session.message_count());

    // Verify context was maintained
    let response_lower = response.to_lowercase();
    if response_lower.contains("ecila") || response_lower.contains("e-c-i-l-a") {
        println!("✅ Context maintained correctly across runs!");
    } else {
        println!("⚠️ Context may not be fully maintained (expected 'ecilA')");
    }

    Ok(())
}

async fn test_multi_run_with_tools(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(3);

    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    // Start with initial state
    let mut session = Session::with_initial_state("test-multi-tool", json!({ "counter": 10 }))
        .with_message(Message::system(
            "You are a helpful assistant. Use the counter tool to manage a counter. \
             Always use the tool when asked about the counter.",
        ));

    // Run 1: Get current value
    session = session.with_message(Message::user("What is the current counter value?"));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Run 1 - User: What is the current counter value?");
    println!("Run 1 - Assistant: {}", response);

    // Run 2: Increment
    session = session.with_message(Message::user("Add 5 to it."));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Run 2 - User: Add 5 to it.");
    println!("Run 2 - Assistant: {}", response);

    // Run 3: Decrement
    session = session.with_message(Message::user("Now subtract 3."));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Run 3 - User: Now subtract 3.");
    println!("Run 3 - Assistant: {}", response);

    // Run 4: Verify final state
    session = session.with_message(Message::user("What is the final value?"));

    let (session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("Run 4 - User: What is the final value?");
    println!("Run 4 - Assistant: {}", response);

    // Check state
    let final_state = session.rebuild_state()?;
    let final_counter = final_state["counter"].as_i64().unwrap_or(-1);

    println!("\nSession summary:");
    println!("  Total messages: {}", session.message_count());
    println!("  Total patches: {}", session.patch_count());
    println!(
        "  Final counter: {} (expected: 12 = 10 + 5 - 3)",
        final_counter
    );

    if final_counter == 12 {
        println!("✅ Multi-run with tools working correctly!");
    } else {
        println!(
            "⚠️ Final counter value unexpected (got {}, expected 12)",
            final_counter
        );
    }

    Ok(())
}

async fn test_session_persistence(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(3);

    // Create a temporary directory for storage
    let temp_dir = std::env::temp_dir().join(format!("carve-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;
    let storage = FileStorage::new(&temp_dir);

    println!("Storage path: {:?}", temp_dir);

    // Create tools
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    // ========== Phase 1: Create session and have a conversation ==========
    println!("\n[Phase 1: Initial conversation]");

    let mut session = Session::with_initial_state("persist-test", json!({ "counter": 100 }))
        .with_message(Message::system(
            "You are a helpful assistant. Use the counter tool. Keep responses brief.",
        ))
        .with_message(Message::user(
            "My favorite number is 42. Remember it. Also, what is the counter?",
        ));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("User: My favorite number is 42. Remember it. Also, what is the counter?");
    println!("Assistant: {}", response);

    // Add another step
    session = session.with_message(Message::user(
        "Increment the counter by my favorite number.",
    ));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("User: Increment the counter by my favorite number.");
    println!("Assistant: {}", response);

    // Save session
    storage.save(&session).await?;
    println!("\n✅ Session saved to disk");
    println!("   Messages: {}", session.message_count());
    println!("   Patches: {}", session.patch_count());

    let state_before = session.rebuild_state()?;
    println!("   State: counter = {}", state_before["counter"]);

    // ========== Phase 2: Load session and continue conversation ==========
    println!("\n[Phase 2: Load and continue]");

    // Load the session (simulating app restart)
    let loaded_session = storage
        .load("persist-test")
        .await?
        .ok_or("Session not found")?;

    println!("✅ Session loaded from disk");
    println!("   Messages: {}", loaded_session.message_count());
    println!("   Patches: {}", loaded_session.patch_count());

    let state_after_load = loaded_session.rebuild_state()?;
    println!("   State: counter = {}", state_after_load["counter"]);

    // Verify state matches
    assert_eq!(
        state_before["counter"], state_after_load["counter"],
        "State mismatch after load!"
    );

    // Continue conversation with loaded session
    let mut session = loaded_session;
    session = session.with_message(Message::user(
        "What was my favorite number? And what is the counter now?",
    ));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("\nUser: What was my favorite number? And what is the counter now?");
    println!("Assistant: {}", response);

    // Verify context was preserved
    let response_lower = response.to_lowercase();
    let remembers_42 = response_lower.contains("42");
    let knows_counter = response_lower.contains("142") || response_lower.contains("counter");

    println!("\n[Verification]");
    if remembers_42 {
        println!("✅ Remembered favorite number (42)");
    } else {
        println!("⚠️ Did not mention favorite number 42");
    }

    if knows_counter {
        println!("✅ Knows current counter value");
    } else {
        println!("⚠️ Counter value unclear");
    }

    // Final state check
    let final_state = session.rebuild_state()?;
    let final_counter = final_state["counter"].as_i64().unwrap_or(-1);
    println!(
        "\nFinal state: counter = {} (expected: 142 = 100 + 42)",
        final_counter
    );

    if final_counter == 142 {
        println!("✅ Session persistence and restore working correctly!");
    } else {
        println!("⚠️ Counter value unexpected");
    }

    // Cleanup
    std::fs::remove_dir_all(&temp_dir)?;
    println!("\n✅ Cleanup completed");

    Ok(())
}

/// Test 8: Parallel tool calls - LLM calls multiple tools at once
async fn test_parallel_tool_calls(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(3)
        .with_parallel_tools(true);

    // Create multiple independent tools
    let calc_tool: Arc<dyn Tool> = Arc::new(CalculatorTool);
    let weather_tool: Arc<dyn Tool> = Arc::new(WeatherTool);
    let tools = tool_map_from_arc([calc_tool, weather_tool]);

    let session = Session::new("test-parallel")
        .with_message(Message::system(
            "You are a helpful assistant with access to a calculator and weather tool. \
             When asked to do multiple independent things, call multiple tools in parallel.",
        ))
        .with_message(Message::user(
            "Please do these two things at the same time: \
             1) Calculate 15 * 8 + 20 \
             2) Get the weather in Tokyo",
        ));

    let (session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("User: Calculate 15*8+20 AND get Tokyo weather (at the same time)");
    println!("Assistant: {}", response);

    // Count how many tool calls were made
    let mut tool_call_count = 0;
    let mut tools_in_single_message = 0;
    for msg in &session.messages {
        if let Some(ref calls) = msg.tool_calls {
            let count = calls.len();
            tool_call_count += count;
            if count > 1 {
                tools_in_single_message = count;
            }
            for call in calls {
                println!("  Tool called: {} with args: {}", call.name, call.arguments);
            }
        }
    }

    println!("\nResults:");
    println!("  Total tool calls: {}", tool_call_count);
    println!("  Tools in single message: {}", tools_in_single_message);

    if tools_in_single_message >= 2 {
        println!(
            "✅ Parallel tool calls working! LLM called {} tools at once.",
            tools_in_single_message
        );
    } else if tool_call_count >= 2 {
        println!(
            "⚠️ Tools were called sequentially ({} total calls)",
            tool_call_count
        );
    } else {
        println!("⚠️ Expected at least 2 tool calls");
    }

    Ok(())
}

/// Test 9: Max rounds limit - ensure infinite loops are prevented
async fn test_max_rounds_limit(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let max_rounds = 3;
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(max_rounds);

    let loop_tool: Arc<dyn Tool> = Arc::new(InfiniteLoopTool);
    let tools = tool_map_from_arc([loop_tool]);

    let session = Session::new("test-max-rounds")
        .with_message(Message::system(
            "You are a system monitor. Use the check_status tool to check all components. \
             Keep checking until all components report 'ok'. Always follow the tool's suggestions.",
        ))
        .with_message(Message::user(
            "Check the status of the 'api' component completely.",
        ));

    let result = run_loop(client, &config, session, &tools).await;

    match result {
        Ok((session, response)) => {
            println!("User: Check the status (triggers potential infinite loop)");
            println!("Assistant: {}", response);
            println!("Messages: {}", session.message_count());

            // Count tool calls
            let tool_calls: usize = session
                .messages
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .map(|c| c.len())
                .sum();
            println!("Tool calls made: {}", tool_calls);

            if tool_calls <= max_rounds as usize {
                println!(
                    "✅ Max rounds limit respected (stopped at {} rounds)",
                    tool_calls
                );
            } else {
                println!(
                    "⚠️ Made {} tool calls, expected <= {}",
                    tool_calls, max_rounds
                );
            }
        }
        Err(AgentLoopError::MaxRoundsExceeded(rounds)) => {
            println!(
                "✅ MaxRoundsExceeded error thrown after {} rounds (expected!)",
                rounds
            );
        }
        Err(e) => {
            println!("Unexpected error: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}

/// Test 10: Tool failure recovery - LLM handles tool errors gracefully
async fn test_tool_failure_recovery(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(5);

    // Tool that fails twice then succeeds
    let unreliable_tool: Arc<dyn Tool> = Arc::new(UnreliableTool::new(2));
    let tools = tool_map_from_arc([unreliable_tool]);

    let session = Session::new("test-failure-recovery")
        .with_message(Message::system(
            "You are a helpful assistant. Use the unreliable_api tool to process queries. \
             If the tool fails, try again. The API may need multiple attempts.",
        ))
        .with_message(Message::user(
            "Please use the API to process the query 'hello world'.",
        ));

    let (session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("User: Process 'hello world' with unreliable API");
    println!("Assistant: {}", response);

    // Count tool calls and check for error messages
    let mut tool_calls = 0;
    let mut error_messages = 0;
    for msg in &session.messages {
        if let Some(ref calls) = msg.tool_calls {
            tool_calls += calls.len();
        }
        if msg.role == carve_agent::Role::Tool && msg.content.contains("error") {
            error_messages += 1;
        }
    }

    println!("\nResults:");
    println!("  Total tool calls: {}", tool_calls);
    println!("  Error responses: {}", error_messages);

    if response.to_lowercase().contains("success") || response.to_lowercase().contains("processed")
    {
        println!("✅ LLM recovered from tool failures and completed the task!");
    } else if error_messages > 0 {
        println!("⚠️ Tool failed but LLM handled it gracefully");
    } else {
        println!("⚠️ Unexpected behavior");
    }

    Ok(())
}

/// Test 11: Session snapshot - collapse patches and continue
async fn test_session_snapshot(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(5);

    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    // Phase 1: Create session with multiple operations
    println!("[Phase 1: Build up patches]");
    let mut session = Session::with_initial_state("test-snapshot", json!({ "counter": 0 }))
        .with_message(Message::system(
            "You are a helpful assistant. Use the counter tool.",
        ))
        .with_message(Message::user(
            "Increment the counter 3 times by 10 each time.",
        ));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("After increments:");
    println!("  Assistant: {}", response);
    println!("  Patches: {}", session.patch_count());
    println!("  State: {:?}", session.rebuild_state()?);

    // Phase 2: Snapshot to collapse patches
    println!("\n[Phase 2: Snapshot]");
    let patches_before = session.patch_count();
    let session = session.snapshot()?;
    let patches_after = session.patch_count();

    println!("  Patches before snapshot: {}", patches_before);
    println!("  Patches after snapshot: {}", patches_after);
    println!("  State in session: {:?}", session.state);

    // Phase 3: Continue after snapshot
    println!("\n[Phase 3: Continue after snapshot]");
    let session = session.with_message(Message::user("What is the counter now? Then add 5 more."));

    let (session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    let final_state = session.rebuild_state()?;
    let final_counter = final_state["counter"].as_i64().unwrap_or(-1);

    println!("  Assistant: {}", response);
    println!("  Final counter: {} (expected: 35 = 30 + 5)", final_counter);

    if patches_after == 0 {
        println!("✅ Snapshot collapsed patches to 0");
    }
    if final_counter >= 30 {
        println!("✅ Session continued correctly after snapshot");
    }

    Ok(())
}

/// Test 12: State replay - go back to a previous state point
async fn test_state_replay(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(3);

    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    // Build session with multiple state changes
    println!("[Building state history]");
    let mut session = Session::with_initial_state("test-replay", json!({ "counter": 0 }))
        .with_message(Message::system(
            "You are a helpful assistant. Use the counter tool.",
        ));

    // Run 1: Set counter to 10
    session = session.with_message(Message::user("Set the counter to 10."));
    let (new_session, _) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    let state_after_1 = session.rebuild_state()?;
    let patches_after_1 = session.patch_count();
    println!(
        "After run 1: counter = {}, patches = {}",
        state_after_1["counter"], patches_after_1
    );

    // Run 2: Add 20
    session = session.with_message(Message::user("Now add 20 to the counter."));
    let (new_session, _) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    let state_after_2 = session.rebuild_state()?;
    let patches_after_2 = session.patch_count();
    println!(
        "After run 2: counter = {}, patches = {}",
        state_after_2["counter"], patches_after_2
    );

    // Run 3: Add 30
    session = session.with_message(Message::user("Add 30 more."));
    let (new_session, _) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    let state_after_3 = session.rebuild_state()?;
    let patches_after_3 = session.patch_count();
    println!(
        "After run 3: counter = {}, patches = {}",
        state_after_3["counter"], patches_after_3
    );

    // Now replay to earlier states
    println!("\n[Replaying to earlier states]");

    let total_patches = session.patch_count();
    if total_patches > 0 {
        // Replay through each patch point
        for i in 0..total_patches {
            let replayed_state = session.replay_to(i)?;
            println!(
                "  State at patch {}: counter = {}",
                i, replayed_state["counter"]
            );
        }

        // Show final state for comparison
        let final_state = session.rebuild_state()?;
        println!("  Final state: counter = {}", final_state["counter"]);

        println!(
            "\n✅ State replay working! Can access {} historical state points.",
            total_patches
        );
    } else {
        println!("⚠️ No patches recorded (LLM may not have used the tool)");
    }

    Ok(())
}

/// Test 13: Long conversation performance
async fn test_long_conversation(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    println!("[Building long conversation history]");
    let start_time = std::time::Instant::now();

    // Create a session with many messages
    let mut session = Session::new("test-long-conv").with_message(Message::system(
        "You are a helpful assistant. Keep answers brief.",
    ));

    // Add 20 runs of conversation history
    for i in 1..=20 {
        session = session
            .with_message(Message::user(format!(
                "Message {} from user. Remember the number {}.",
                i,
                i * 10
            )))
            .with_message(Message::assistant(format!(
                "I understand, message {}. The number is {}.",
                i,
                i * 10
            )));
    }

    let build_time = start_time.elapsed();
    println!(
        "  Built {} messages in {:?}",
        session.message_count(),
        build_time
    );

    // Now send a new message that requires remembering context
    session = session.with_message(Message::user(
        "What was the number I mentioned in message 15? Reply with just the number.",
    ));

    let request_start = std::time::Instant::now();
    let (session, response) = run_loop(client, &config, session, &std::collections::HashMap::new())
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    let request_time = request_start.elapsed();

    println!("\n[Results]");
    println!("  Total messages: {}", session.message_count());
    println!("  Request time: {:?}", request_time);
    println!("  Assistant: {}", response);

    // Check if LLM remembered the context
    if response.contains("150") {
        println!("✅ LLM correctly remembered context from long conversation!");
    } else {
        println!("⚠️ LLM may not have processed full context (expected '150')");
    }

    // Test session serialization performance with large history
    let storage = carve_agent::MemoryStorage::new();
    let save_start = std::time::Instant::now();
    storage.save(&session).await?;
    let save_time = save_start.elapsed();

    let load_start = std::time::Instant::now();
    let loaded = storage.load("test-long-conv").await?.unwrap();
    let load_time = load_start.elapsed();

    println!("\n[Storage Performance]");
    println!("  Save time: {:?}", save_time);
    println!("  Load time: {:?}", load_time);
    println!("  Loaded messages: {}", loaded.message_count());

    if save_time.as_millis() < 100 && load_time.as_millis() < 100 {
        println!("✅ Storage performance good (<100ms for save/load)");
    }

    Ok(())
}
