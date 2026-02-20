//! Live test with DeepSeek API.
//!
//! Run with:
//! ```bash
//! export DEEPSEEK_API_KEY=<your-key>
//! cargo run --example live_deepseek
//! ```

use async_trait::async_trait;
use tirea_agent_loop::contracts::AgentEvent;
use tirea_agent_loop::contracts::thread::Thread as ConversationAgentState;
use tirea_agent_loop::contracts::thread::Message;
use tirea_agent_loop::contracts::storage::{AgentStateReader, AgentStateWriter};
use tirea_agent_loop::contracts::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use tirea_agent_loop::contracts::ToolCallContext;
use tirea_agent_loop::contracts::RunContext;
use tirea_agent_loop::runtime::loop_runner::{
    run_loop, run_loop_stream, tool_map_from_arc, AgentConfig,
};
use tirea_state::State;
use tirea_store_adapters::{FileStore, MemoryStore};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::collections::HashMap;
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

    async fn execute(&self, args: Value, _ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
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

    async fn execute(&self, args: Value, _ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
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

    async fn execute(&self, args: Value, _ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
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

    async fn execute(&self, args: Value, _ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
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

    async fn execute(&self, args: Value, ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
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

    // Create tools
    let calc_tool: Arc<dyn Tool> = Arc::new(CalculatorTool);
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([calc_tool, counter_tool]);

    // Test 1: Simple conversation (no tools)
    println!("--- Test 1: Simple Conversation ---");
    test_simple_conversation().await?;

    // Test 2: Tool calling
    println!("\n--- Test 2: Calculator Tool ---");
    test_calculator(&tools).await?;

    // Test 3: Streaming
    println!("\n--- Test 3: Streaming Response ---");
    test_streaming().await?;

    // Test 4: Tool with state
    println!("\n--- Test 4: Counter Tool with State ---");
    test_counter_with_state().await?;

    // Test 5: Multi-run conversation
    println!("\n--- Test 5: Multi-run Conversation ---");
    test_multi_run().await?;

    // Test 6: Multi-run with tools
    println!("\n--- Test 6: Multi-run with Tools ---");
    test_multi_run_with_tools().await?;

    // Test 7: Thread persistence and restore
    println!("\n--- Test 7: Thread Persistence & Restore ---");
    test_session_persistence().await?;

    // Test 8: Parallel tool calls
    println!("\n--- Test 8: Parallel Tool Calls ---");
    test_parallel_tool_calls().await?;

    // Test 9: Max rounds limit
    println!("\n--- Test 9: Max Rounds Limit ---");
    test_max_rounds_limit().await?;

    // Test 10: Tool failure recovery
    println!("\n--- Test 10: Tool Failure Recovery ---");
    test_tool_failure_recovery().await?;

    // Test 11: Thread snapshot continuation
    println!("\n--- Test 11: Thread Snapshot Continuation ---");
    test_session_snapshot().await?;

    // Test 12: State replay
    println!("\n--- Test 12: State Replay ---");
    test_state_replay().await?;

    // Test 13: Long conversation performance
    println!("\n--- Test 13: Long Conversation Performance ---");
    test_long_conversation().await?;

    println!("\n=== All tests completed! ===");
    Ok(())
}

async fn test_simple_conversation() -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    let thread = ConversationAgentState::new("test-simple")
        .with_message(Message::user("What is 2 + 2? Reply briefly."));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, HashMap::new(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: What is 2 + 2? Reply briefly.");
    println!("Assistant: {}", response);
    println!("Messages in thread: {}", outcome.run_ctx.messages().len());

    Ok(())
}

async fn test_calculator(
    tools: &std::collections::HashMap<String, Arc<dyn Tool>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(3);

    let thread = ConversationAgentState::new("test-calc")
        .with_message(Message::system("You are a helpful assistant. Use the calculator tool when asked to perform calculations."))
        .with_message(Message::user("Please calculate 15 * 7 + 23 using the calculator tool."));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: Please calculate 15 * 7 + 23 using the calculator tool.");
    println!("Assistant: {}", response);
    println!("Messages in thread: {}", outcome.run_ctx.messages().len());
    println!("Patches in thread: {}", outcome.run_ctx.thread_patches().len());

    // Show tool calls from messages
    for msg in outcome.run_ctx.messages() {
        if let Some(ref calls) = msg.tool_calls {
            for call in calls {
                println!("Tool called: {} with args: {}", call.name, call.arguments);
            }
        }
    }

    Ok(())
}

async fn test_streaming() -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    let thread = ConversationAgentState::new("test-stream")
        .with_message(Message::user("Count from 1 to 5, one number per line."));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;

    println!("User: Count from 1 to 5, one number per line.");
    print!("Assistant: ");

    let mut stream = run_loop_stream(config, HashMap::new(), run_ctx, None, None);

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

async fn test_counter_with_state() -> Result<(), Box<dyn std::error::Error>> {
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(5);

    // Create session with initial state
    let thread = ConversationAgentState::with_initial_state("test-counter", json!({ "counter": 0 }))
        .with_message(Message::system("You are a helpful assistant. Use the counter tool to manage a counter. Always use the tool when asked about the counter."))
        .with_message(Message::user("Please increment the counter by 5, then increment it by 3 more. Tell me the final value."));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: Please increment the counter by 5, then increment it by 3 more. Tell me the final value.");
    println!("Assistant: {}", response);
    println!("Messages in thread: {}", outcome.run_ctx.messages().len());
    println!("Patches in thread: {}", outcome.run_ctx.thread_patches().len());

    // Rebuild state to see final counter value
    let final_state = outcome.run_ctx.snapshot()?;
    println!(
        "Final state: {}",
        serde_json::to_string_pretty(&final_state)?
    );

    Ok(())
}

async fn test_multi_run() -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    // Run 1: Introduce a topic
    let thread = ConversationAgentState::new("test-multi-run")
        .with_message(Message::system(
            "You are a helpful assistant. Keep your answers brief.",
        ))
        .with_message(Message::user("My name is Alice. Remember it."));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, HashMap::new(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("Run 1 - User: My name is Alice. Remember it.");
    println!("Run 1 - Assistant: {}", response);

    // Run 2: Test if context is remembered
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user("What is my name?")));

    let outcome = run_loop(&config, HashMap::new(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("Run 2 - User: What is my name?");
    println!("Run 2 - Assistant: {}", response);

    // Run 3: Another follow-up
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user("Say my name backwards.")));

    let outcome = run_loop(&config, HashMap::new(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("Run 3 - User: Say my name backwards.");
    println!("Run 3 - Assistant: {}", response);
    println!("Total messages in thread: {}", outcome.run_ctx.messages().len());

    // Verify context was maintained
    let response_lower = response.to_lowercase();
    if response_lower.contains("ecila") || response_lower.contains("e-c-i-l-a") {
        println!("Context maintained correctly across runs!");
    } else {
        println!("Context may not be fully maintained (expected 'ecilA')");
    }

    Ok(())
}

async fn test_multi_run_with_tools() -> Result<(), Box<dyn std::error::Error>> {
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(3);

    // Start with initial state
    let thread =
        ConversationAgentState::with_initial_state("test-multi-tool", json!({ "counter": 10 }))
            .with_message(Message::system(
                "You are a helpful assistant. Use the counter tool to manage a counter. \
             Always use the tool when asked about the counter.",
            ))
            .with_message(Message::user("What is the current counter value?"));

    // Run 1: Get current value
    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("Run 1 - User: What is the current counter value?");
    println!("Run 1 - Assistant: {}", response);

    // Run 2: Increment
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user("Add 5 to it.")));

    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("Run 2 - User: Add 5 to it.");
    println!("Run 2 - Assistant: {}", response);

    // Run 3: Decrement
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user("Now subtract 3.")));

    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("Run 3 - User: Now subtract 3.");
    println!("Run 3 - Assistant: {}", response);

    // Run 4: Verify final state
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user("What is the final value?")));

    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("Run 4 - User: What is the final value?");
    println!("Run 4 - Assistant: {}", response);

    // Check state
    let final_state = outcome.run_ctx.snapshot()?;
    let final_counter = final_state["counter"].as_i64().unwrap_or(-1);

    println!("\nSession summary:");
    println!("  Total messages: {}", outcome.run_ctx.messages().len());
    println!("  Total patches: {}", outcome.run_ctx.thread_patches().len());
    println!(
        "  Final counter: {} (expected: 12 = 10 + 5 - 3)",
        final_counter
    );

    if final_counter == 12 {
        println!("Multi-run with tools working correctly!");
    } else {
        println!(
            "Final counter value unexpected (got {}, expected 12)",
            final_counter
        );
    }

    Ok(())
}

async fn test_session_persistence() -> Result<(), Box<dyn std::error::Error>> {
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(3);

    // Create a temporary directory for storage
    let temp_dir = std::env::temp_dir().join(format!("tirea-test-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;
    let storage = FileStore::new(&temp_dir);

    println!("Storage path: {:?}", temp_dir);

    // ========== Phase 1: Create session and have a conversation ==========
    println!("\n[Phase 1: Initial conversation]");

    let thread =
        ConversationAgentState::with_initial_state("persist-test", json!({ "counter": 100 }))
            .with_message(Message::system(
                "You are a helpful assistant. Use the counter tool. Keep responses brief.",
            ))
            .with_message(Message::user(
                "My favorite number is 42. Remember it. Also, what is the counter?",
            ));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: My favorite number is 42. Remember it. Also, what is the counter?");
    println!("Assistant: {}", response);

    // Add another step
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user(
        "Increment the counter by my favorite number.",
    )));

    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: Increment the counter by my favorite number.");
    println!("Assistant: {}", response);

    // For persistence we need a Thread. Rebuild one from the original thread
    // by replaying messages from the run_ctx.
    // Since Thread persistence operations need a Thread, we build one from the
    // original thread's initial state and append messages from run_ctx.
    let mut persist_thread = ConversationAgentState::with_initial_state("persist-test", json!({ "counter": 100 }));
    for msg in outcome.run_ctx.messages() {
        persist_thread = persist_thread.with_message((*msg.as_ref()).clone());
    }
    // Copy patches
    for tp in outcome.run_ctx.thread_patches() {
        persist_thread.patches.push(tp.clone());
    }

    // Save session
    storage.save(&persist_thread).await?;
    println!("\nThread saved to disk");
    println!("   Messages: {}", persist_thread.message_count());
    println!("   Patches: {}", persist_thread.patch_count());

    let state_before = persist_thread.rebuild_state()?;
    println!("   State: counter = {}", state_before["counter"]);

    // ========== Phase 2: Load session and continue conversation ==========
    println!("\n[Phase 2: Load and continue]");

    // Load the session (simulating app restart)
    let loaded_thread = storage
        .load_agent_state("persist-test")
        .await?
        .ok_or("Thread not found")?;

    println!("Thread loaded from disk");
    println!("   Messages: {}", loaded_thread.message_count());
    println!("   Patches: {}", loaded_thread.patch_count());

    let state_after_load = loaded_thread.rebuild_state()?;
    println!("   State: counter = {}", state_after_load["counter"]);

    // Verify state matches
    assert_eq!(
        state_before["counter"], state_after_load["counter"],
        "State mismatch after load!"
    );

    // Continue conversation with loaded session
    let mut loaded_thread = loaded_thread;
    loaded_thread = loaded_thread.with_message(Message::user(
        "What was my favorite number? And what is the counter now?",
    ));

    let run_ctx = RunContext::from_thread(&loaded_thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("\nUser: What was my favorite number? And what is the counter now?");
    println!("Assistant: {}", response);

    // Verify context was preserved
    let response_lower = response.to_lowercase();
    let remembers_42 = response_lower.contains("42");
    let knows_counter = response_lower.contains("142") || response_lower.contains("counter");

    println!("\n[Verification]");
    if remembers_42 {
        println!("Remembered favorite number (42)");
    } else {
        println!("Did not mention favorite number 42");
    }

    if knows_counter {
        println!("Knows current counter value");
    } else {
        println!("Counter value unclear");
    }

    // Final state check
    let final_state = outcome.run_ctx.snapshot()?;
    let final_counter = final_state["counter"].as_i64().unwrap_or(-1);
    println!(
        "\nFinal state: counter = {} (expected: 142 = 100 + 42)",
        final_counter
    );

    if final_counter == 142 {
        println!("Thread persistence and restore working correctly!");
    } else {
        println!("Counter value unexpected");
    }

    // Cleanup
    std::fs::remove_dir_all(&temp_dir)?;
    println!("\nCleanup completed");

    Ok(())
}

/// Test 8: Parallel tool calls - LLM calls multiple tools at once
async fn test_parallel_tool_calls() -> Result<(), Box<dyn std::error::Error>> {
    // Create multiple independent tools
    let calc_tool: Arc<dyn Tool> = Arc::new(CalculatorTool);
    let weather_tool: Arc<dyn Tool> = Arc::new(WeatherTool);
    let tools = tool_map_from_arc([calc_tool, weather_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(3)
        .with_parallel_tools(true);

    let thread = ConversationAgentState::new("test-parallel")
        .with_message(Message::system(
            "You are a helpful assistant with access to a calculator and weather tool. \
             When asked to do multiple independent things, call multiple tools in parallel.",
        ))
        .with_message(Message::user(
            "Please do these two things at the same time: \
             1) Calculate 15 * 8 + 20 \
             2) Get the weather in Tokyo",
        ));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: Calculate 15*8+20 AND get Tokyo weather (at the same time)");
    println!("Assistant: {}", response);

    // Count how many tool calls were made
    let mut tool_call_count = 0;
    let mut tools_in_single_message = 0;
    for msg in outcome.run_ctx.messages() {
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
            "Parallel tool calls working! LLM called {} tools at once.",
            tools_in_single_message
        );
    } else if tool_call_count >= 2 {
        println!(
            "Tools were called sequentially ({} total calls)",
            tool_call_count
        );
    } else {
        println!("Expected at least 2 tool calls");
    }

    Ok(())
}

/// Test 9: Max rounds limit - ensure infinite loops are prevented
async fn test_max_rounds_limit() -> Result<(), Box<dyn std::error::Error>> {
    let max_rounds = 3;

    let loop_tool: Arc<dyn Tool> = Arc::new(InfiniteLoopTool);
    let tools = tool_map_from_arc([loop_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(max_rounds);

    let thread = ConversationAgentState::new("test-max-rounds")
        .with_message(Message::system(
            "You are a system monitor. Use the check_status tool to check all components. \
             Keep checking until all components report 'ok'. Always follow the tool's suggestions.",
        ))
        .with_message(Message::user(
            "Check the status of the 'api' component completely.",
        ));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: Check the status (triggers potential infinite loop)");
    println!("Assistant: {}", response);
    println!("Messages: {}", outcome.run_ctx.messages().len());

    // Count tool calls
    let tool_calls: usize = outcome
        .run_ctx
        .messages()
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .map(|c| c.len())
        .sum();
    println!("Tool calls made: {}", tool_calls);

    if tool_calls <= max_rounds {
        println!(
            "Max rounds limit respected (stopped at {} rounds)",
            tool_calls
        );
    } else {
        println!(
            "Made {} tool calls, expected <= {}",
            tool_calls, max_rounds
        );
    }

    Ok(())
}

/// Test 10: Tool failure recovery - LLM handles tool errors gracefully
async fn test_tool_failure_recovery() -> Result<(), Box<dyn std::error::Error>> {
    // Tool that fails twice then succeeds
    let unreliable_tool: Arc<dyn Tool> = Arc::new(UnreliableTool::new(2));
    let tools = tool_map_from_arc([unreliable_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(5);

    let thread = ConversationAgentState::new("test-failure-recovery")
        .with_message(Message::system(
            "You are a helpful assistant. Use the unreliable_api tool to process queries. \
             If the tool fails, try again. The API may need multiple attempts.",
        ))
        .with_message(Message::user(
            "Please use the API to process the query 'hello world'.",
        ));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("User: Process 'hello world' with unreliable API");
    println!("Assistant: {}", response);

    // Count tool calls and check for error messages
    let mut tool_calls = 0;
    let mut error_messages = 0;
    for msg in outcome.run_ctx.messages() {
        if let Some(ref calls) = msg.tool_calls {
            tool_calls += calls.len();
        }
        if msg.role == tirea_agent_loop::contracts::thread::Role::Tool
            && msg.content.contains("error")
        {
            error_messages += 1;
        }
    }

    println!("\nResults:");
    println!("  Total tool calls: {}", tool_calls);
    println!("  Error responses: {}", error_messages);

    if response.to_lowercase().contains("success") || response.to_lowercase().contains("processed")
    {
        println!("LLM recovered from tool failures and completed the task!");
    } else if error_messages > 0 {
        println!("Tool failed but LLM handled it gracefully");
    } else {
        println!("Unexpected behavior");
    }

    Ok(())
}

/// Test 11: Thread snapshot - collapse patches and continue
async fn test_session_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(5);

    // Phase 1: Create session with multiple operations
    println!("[Phase 1: Build up patches]");
    let thread =
        ConversationAgentState::with_initial_state("test-snapshot", json!({ "counter": 0 }))
            .with_message(Message::system(
                "You are a helpful assistant. Use the counter tool.",
            ))
            .with_message(Message::user(
                "Increment the counter 3 times by 10 each time.",
            ));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("After increments:");
    println!("  Assistant: {}", response);
    println!("  Patches: {}", outcome.run_ctx.thread_patches().len());
    println!("  State: {:?}", outcome.run_ctx.snapshot()?);

    // Phase 2: Snapshot to collapse patches - need a Thread for this
    println!("\n[Phase 2: Snapshot]");
    let mut snapshot_thread = ConversationAgentState::with_initial_state("test-snapshot", json!({ "counter": 0 }));
    for msg in outcome.run_ctx.messages() {
        snapshot_thread = snapshot_thread.with_message((*msg.as_ref()).clone());
    }
    for tp in outcome.run_ctx.thread_patches() {
        snapshot_thread.patches.push(tp.clone());
    }

    let patches_before = snapshot_thread.patch_count();
    let snapshot_thread = snapshot_thread.snapshot()?;
    let patches_after = snapshot_thread.patch_count();

    println!("  Patches before snapshot: {}", patches_before);
    println!("  Patches after snapshot: {}", patches_after);
    println!("  State in thread: {:?}", snapshot_thread.state);

    // Phase 3: Continue after snapshot
    println!("\n[Phase 3: Continue after snapshot]");
    let snapshot_thread = snapshot_thread.with_message(Message::user("What is the counter now? Then add 5 more."));

    let run_ctx = RunContext::from_thread(&snapshot_thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let response = outcome.response.as_deref().unwrap_or_default();
    let final_state = outcome.run_ctx.snapshot()?;
    let final_counter = final_state["counter"].as_i64().unwrap_or(-1);

    println!("  Assistant: {}", response);
    println!("  Final counter: {} (expected: 35 = 30 + 5)", final_counter);

    if patches_after == 0 {
        println!("Snapshot collapsed patches to 0");
    }
    if final_counter >= 30 {
        println!("Thread continued correctly after snapshot");
    }

    Ok(())
}

/// Test 12: State replay - go back to a previous state point
async fn test_state_replay() -> Result<(), Box<dyn std::error::Error>> {
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(3);

    // Build session with multiple state changes
    println!("[Building state history]");
    let thread =
        ConversationAgentState::with_initial_state("test-replay", json!({ "counter": 0 }))
            .with_message(Message::system(
                "You are a helpful assistant. Use the counter tool.",
            ))
            .with_message(Message::user("Set the counter to 10."));

    // Run 1: Set counter to 10
    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;
    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let state_after_1 = outcome.run_ctx.snapshot()?;
    let patches_after_1 = outcome.run_ctx.thread_patches().len();
    println!(
        "After run 1: counter = {}, patches = {}",
        state_after_1["counter"], patches_after_1
    );

    // Run 2: Add 20
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user("Now add 20 to the counter.")));

    let outcome = run_loop(&config, tools.clone(), run_ctx, None, None).await;

    let state_after_2 = outcome.run_ctx.snapshot()?;
    let patches_after_2 = outcome.run_ctx.thread_patches().len();
    println!(
        "After run 2: counter = {}, patches = {}",
        state_after_2["counter"], patches_after_2
    );

    // Run 3: Add 30
    let mut run_ctx = outcome.run_ctx;
    run_ctx.add_message(Arc::new(Message::user("Add 30 more.")));

    let outcome = run_loop(&config, tools, run_ctx, None, None).await;

    let state_after_3 = outcome.run_ctx.snapshot()?;
    let patches_after_3 = outcome.run_ctx.thread_patches().len();
    println!(
        "After run 3: counter = {}, patches = {}",
        state_after_3["counter"], patches_after_3
    );

    // For replay we need a Thread - reconstruct one
    let mut replay_thread = ConversationAgentState::with_initial_state("test-replay", json!({ "counter": 0 }));
    for msg in outcome.run_ctx.messages() {
        replay_thread = replay_thread.with_message((*msg.as_ref()).clone());
    }
    for tp in outcome.run_ctx.thread_patches() {
        replay_thread.patches.push(tp.clone());
    }

    // Now replay to earlier states
    println!("\n[Replaying to earlier states]");

    let total_patches = replay_thread.patch_count();
    if total_patches > 0 {
        // Replay through each patch point
        for i in 0..total_patches {
            let replayed_state = replay_thread.replay_to(i)?;
            println!(
                "  State at patch {}: counter = {}",
                i, replayed_state["counter"]
            );
        }

        // Show final state for comparison
        let final_state = replay_thread.rebuild_state()?;
        println!("  Final state: counter = {}", final_state["counter"]);

        println!(
            "\nState replay working! Can access {} historical state points.",
            total_patches
        );
    } else {
        println!("No patches recorded (LLM may not have used the tool)");
    }

    Ok(())
}

/// Test 13: Long conversation performance
async fn test_long_conversation() -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    println!("[Building long conversation history]");
    let start_time = std::time::Instant::now();

    // Create a session with many messages
    let mut thread = ConversationAgentState::new("test-long-conv").with_message(Message::system(
        "You are a helpful assistant. Keep answers brief.",
    ));

    // Add 20 runs of conversation history
    for i in 1..=20 {
        thread = thread
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
        thread.message_count(),
        build_time
    );

    // Now send a new message that requires remembering context
    thread = thread.with_message(Message::user(
        "What was the number I mentioned in message 15? Reply with just the number.",
    ));

    let run_ctx = RunContext::from_thread(&thread, tirea_contract::RunConfig::default())?;

    let request_start = std::time::Instant::now();
    let outcome = run_loop(&config, HashMap::new(), run_ctx, None, None).await;
    let request_time = request_start.elapsed();

    let response = outcome.response.as_deref().unwrap_or_default();
    println!("\n[Results]");
    println!("  Total messages: {}", outcome.run_ctx.messages().len());
    println!("  Request time: {:?}", request_time);
    println!("  Assistant: {}", response);

    // Check if LLM remembered the context
    if response.contains("150") {
        println!("LLM correctly remembered context from long conversation!");
    } else {
        println!("LLM may not have processed full context (expected '150')");
    }

    // Test session serialization performance with large history
    // Need Thread for storage ops
    let mut storage_thread = ConversationAgentState::new("test-long-conv");
    for msg in outcome.run_ctx.messages() {
        storage_thread = storage_thread.with_message((*msg.as_ref()).clone());
    }

    let storage = MemoryStore::new();
    let save_start = std::time::Instant::now();
    storage.save(&storage_thread).await?;
    let save_time = save_start.elapsed();

    let load_start = std::time::Instant::now();
    let loaded = storage.load_agent_state("test-long-conv").await?.unwrap();
    let load_time = load_start.elapsed();

    println!("\n[Storage Performance]");
    println!("  Save time: {:?}", save_time);
    println!("  Load time: {:?}", load_time);
    println!("  Loaded messages: {}", loaded.message_count());

    if save_time.as_millis() < 100 && load_time.as_millis() < 100 {
        println!("Storage performance good (<100ms for save/load)");
    }

    Ok(())
}
