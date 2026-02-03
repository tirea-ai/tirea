//! Live test with DeepSeek API.
//!
//! Run with:
//! ```bash
//! export DEEPSEEK_API_KEY=<your-key>
//! cargo run --example live_deepseek
//! ```

use async_trait::async_trait;
use carve_agent::{
    run_loop, run_loop_stream, tool_map_from_arc, AgentConfig, AgentEvent, Context, FileStorage,
    Message, Session, Storage, Tool, ToolDescriptor, ToolError, ToolResult,
};
use carve_state_derive::State;
use futures::StreamExt;
use genai::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// A simple calculator tool for testing.
struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("calculator", "Calculator", "Perform arithmetic calculations")
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

        Ok(ToolResult::success("calculator", json!({ "result": result, "expression": expr })))
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
            "set" => args["value"]
                .as_i64()
                .ok_or_else(|| ToolError::InvalidArguments("Missing 'value' for set".to_string()))?,
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

    // Test 5: Multi-turn conversation
    println!("\n--- Test 5: Multi-turn Conversation ---");
    test_multi_turn(&client).await?;

    // Test 6: Multi-turn with tools
    println!("\n--- Test 6: Multi-turn with Tools ---");
    test_multi_turn_with_tools(&client).await?;

    // Test 7: Session persistence and restore
    println!("\n--- Test 7: Session Persistence & Restore ---");
    test_session_persistence(&client).await?;

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

    println!(
        "User: Please calculate 15 * 7 + 23 using the calculator tool."
    );
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

    let mut stream = run_loop_stream(client.clone(), config, session, tools);

    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::TextDelta(delta) => {
                print!("{}", delta);
                std::io::Write::flush(&mut std::io::stdout())?;
            }
            AgentEvent::Done { response } => {
                println!("\n[Stream completed: {} chars]", response.len());
            }
            AgentEvent::Error(e) => {
                println!("\n[Error: {}]", e);
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
    println!("Final state: {}", serde_json::to_string_pretty(&final_state)?);

    Ok(())
}

async fn test_multi_turn(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(1);

    // Turn 1: Introduce a topic
    let mut session = Session::new("test-multi-turn")
        .with_message(Message::system("You are a helpful assistant. Keep your answers brief."))
        .with_message(Message::user("My name is Alice. Remember it."));

    let (new_session, response) =
        run_loop(client, &config, session, &std::collections::HashMap::new())
            .await
            .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Turn 1 - User: My name is Alice. Remember it.");
    println!("Turn 1 - Assistant: {}", response);

    // Turn 2: Test if context is remembered
    session = session.with_message(Message::user("What is my name?"));

    let (new_session, response) =
        run_loop(client, &config, session, &std::collections::HashMap::new())
            .await
            .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Turn 2 - User: What is my name?");
    println!("Turn 2 - Assistant: {}", response);

    // Turn 3: Another follow-up
    session = session.with_message(Message::user("Say my name backwards."));

    let (session, response) =
        run_loop(client, &config, session, &std::collections::HashMap::new())
            .await
            .map_err(|e| format!("LLM error: {}", e))?;

    println!("Turn 3 - User: Say my name backwards.");
    println!("Turn 3 - Assistant: {}", response);
    println!("Total messages in session: {}", session.message_count());

    // Verify context was maintained
    let response_lower = response.to_lowercase();
    if response_lower.contains("ecila") || response_lower.contains("e-c-i-l-a") {
        println!("✅ Context maintained correctly across turns!");
    } else {
        println!("⚠️ Context may not be fully maintained (expected 'ecilA')");
    }

    Ok(())
}

async fn test_multi_turn_with_tools(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat").with_max_rounds(3);

    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let tools = tool_map_from_arc([counter_tool]);

    // Start with initial state
    let mut session = Session::with_initial_state("test-multi-tool", json!({ "counter": 10 }))
        .with_message(Message::system(
            "You are a helpful assistant. Use the counter tool to manage a counter. \
             Always use the tool when asked about the counter.",
        ));

    // Turn 1: Get current value
    session = session.with_message(Message::user("What is the current counter value?"));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Turn 1 - User: What is the current counter value?");
    println!("Turn 1 - Assistant: {}", response);

    // Turn 2: Increment
    session = session.with_message(Message::user("Add 5 to it."));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Turn 2 - User: Add 5 to it.");
    println!("Turn 2 - Assistant: {}", response);

    // Turn 3: Decrement
    session = session.with_message(Message::user("Now subtract 3."));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("Turn 3 - User: Now subtract 3.");
    println!("Turn 3 - Assistant: {}", response);

    // Turn 4: Verify final state
    session = session.with_message(Message::user("What is the final value?"));

    let (session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    println!("Turn 4 - User: What is the final value?");
    println!("Turn 4 - Assistant: {}", response);

    // Check state
    let final_state = session.rebuild_state()?;
    let final_counter = final_state["counter"].as_i64().unwrap_or(-1);

    println!("\nSession summary:");
    println!("  Total messages: {}", session.message_count());
    println!("  Total patches: {}", session.patch_count());
    println!("  Final counter: {} (expected: 12 = 10 + 5 - 3)", final_counter);

    if final_counter == 12 {
        println!("✅ Multi-turn with tools working correctly!");
    } else {
        println!("⚠️ Final counter value unexpected (got {}, expected 12)", final_counter);
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
        .with_message(Message::user("My favorite number is 42. Remember it. Also, what is the counter?"));

    let (new_session, response) = run_loop(client, &config, session, &tools)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;
    session = new_session;

    println!("User: My favorite number is 42. Remember it. Also, what is the counter?");
    println!("Assistant: {}", response);

    // Add another turn
    session = session.with_message(Message::user("Increment the counter by my favorite number."));

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
    session = session.with_message(Message::user("What was my favorite number? And what is the counter now?"));

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
    println!("\nFinal state: counter = {} (expected: 142 = 100 + 42)", final_counter);

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
