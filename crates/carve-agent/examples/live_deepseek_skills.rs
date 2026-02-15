//! Live test: skills + DeepSeek API.
//!
//! Demonstrates the skill subsystem with real LLM calls. Two filesystem skills
//! (unit-converter, todo-manager) are discovered from `examples/skills/` and
//! wired into the agent alongside regular tools.
//!
//! Run with:
//! ```bash
//! export DEEPSEEK_API_KEY=<your-key>
//! cargo run --example live_deepseek_skills
//! ```

use async_trait::async_trait;
use carve_agent::contracts::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agent::extensions::skills::{FsSkillRegistry, SkillSubsystem};
use carve_agent::prelude::Context;
use carve_agent::runtime::loop_runner::{
    run_loop, run_loop_stream, tool_map_from_arc, AgentConfig, RunContext,
};
use carve_agent::runtime::streaming::AgentEvent;
use carve_agent::thread::Thread;
use carve_agent::types::Message;
use carve_state_derive::State;
use futures::StreamExt;
use genai::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Tools (reused from live_deepseek example, kept minimal)
// ---------------------------------------------------------------------------

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

        let result = eval_simple_expr(expr);
        Ok(ToolResult::success(
            "calculator",
            json!({ "result": result, "expression": expr }),
        ))
    }
}

fn eval_simple_expr(expr: &str) -> f64 {
    let expr = expr.replace(' ', "");
    if let Ok(n) = expr.parse::<f64>() {
        return n;
    }
    for op in ['+', '-', '*', '/'] {
        if let Some(pos) = expr.rfind(op) {
            if pos > 0 {
                let l = eval_simple_expr(&expr[..pos]);
                let r = eval_simple_expr(&expr[pos + 1..]);
                return match op {
                    '+' => l + r,
                    '-' => l - r,
                    '*' => l * r,
                    '/' if r != 0.0 => l / r,
                    _ => f64::NAN,
                };
            }
        }
    }
    0.0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
struct CounterState {
    counter: i64,
}

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

        let state = ctx.state::<CounterState>("");
        let current = state.counter().unwrap_or(0);

        let new_value = match action {
            "get" => {
                return Ok(ToolResult::success(
                    "counter",
                    json!({ "value": current, "action": "get" }),
                ));
            }
            "increment" => current + args["value"].as_i64().unwrap_or(1),
            "decrement" => current - args["value"].as_i64().unwrap_or(1),
            "set" => args["value"].as_i64().ok_or_else(|| {
                ToolError::InvalidArguments("Missing 'value' for set".to_string())
            })?,
            _ => {
                return Err(ToolError::InvalidArguments(format!(
                    "Unknown action: {action}"
                )))
            }
        };

        state.set_counter(new_value);
        Ok(ToolResult::success(
            "counter",
            json!({ "previous": current, "current": new_value, "action": action }),
        ))
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("DEEPSEEK_API_KEY").is_err() {
        eprintln!("Error: DEEPSEEK_API_KEY environment variable not set");
        eprintln!(
            "Usage: export DEEPSEEK_API_KEY=<your-key> && cargo run --example live_deepseek_skills"
        );
        std::process::exit(1);
    }

    println!("=== DeepSeek Skills Live Test ===\n");

    let client = Client::default();

    // Discover skills from examples/skills/
    let skills_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("skills");
    println!("Skills root: {}\n", skills_root.display());

    let registry = FsSkillRegistry::discover_root(&skills_root)?;
    let subsystem = SkillSubsystem::new(Arc::new(registry));

    // List discovered skills
    println!("Discovered skills:");
    for meta in subsystem.registry().list() {
        println!("  - {} : {}", meta.name, meta.description);
    }
    println!();

    // Build tools: regular tools + skill tools
    let calc_tool: Arc<dyn Tool> = Arc::new(CalculatorTool);
    let counter_tool: Arc<dyn Tool> = Arc::new(CounterTool);
    let mut tools = tool_map_from_arc([calc_tool, counter_tool]);
    subsystem.extend_tools(&mut tools)?;

    println!("Registered tools: {:?}\n", tools.keys().collect::<Vec<_>>());

    // Test 1: Activate a skill and use it
    println!("--- Test 1: Unit Converter Skill ---");
    test_unit_converter(&client, &tools, &subsystem).await?;

    // Test 2: Skill activation via streaming
    println!("\n--- Test 2: Todo Manager Skill (Streaming) ---");
    test_todo_manager_streaming(&client, &tools, &subsystem).await?;

    // Test 3: Multi-skill conversation
    println!("\n--- Test 3: Multi-Skill Conversation ---");
    test_multi_skill(&client, &tools, &subsystem).await?;

    println!("\n=== All skill tests completed! ===");
    Ok(())
}

async fn test_unit_converter(
    client: &Client,
    tools: &std::collections::HashMap<String, Arc<dyn Tool>>,
    subsystem: &SkillSubsystem,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(5)
        .with_plugin(subsystem.plugin());

    let thread = Thread::with_initial_state("test-unit-converter", json!({}))
        .with_message(Message::system(
            "You are a helpful assistant with access to skills and tools. \
             When the user asks about unit conversions, first activate the \
             unit-converter skill using the skill tool, then use its instructions \
             and the calculator tool to perform conversions.",
        ))
        .with_message(Message::user(
            "Convert 100 miles to kilometers. Use the unit-converter skill.",
        ));

    let (thread, response) = run_loop(client, &config, thread, tools)
        .await
        .map_err(|e| format!("LLM error: {e}"))?;

    println!("User: Convert 100 miles to kilometers.");
    println!("Assistant: {response}");
    println!("Messages: {}", thread.message_count());

    // Check if skill was activated via tool calls
    let skill_activated = thread.messages.iter().any(|m| {
        m.tool_calls
            .as_ref()
            .is_some_and(|calls| calls.iter().any(|c| c.name == "skill"))
    });

    if skill_activated {
        println!("  Skill activated: yes");
    } else {
        println!("  Skill activated: no (LLM did not call the skill tool)");
    }

    // Check if calculator was used
    let calc_used = thread.messages.iter().any(|m| {
        m.tool_calls
            .as_ref()
            .is_some_and(|calls| calls.iter().any(|c| c.name == "calculator"))
    });
    println!(
        "  Calculator used: {}",
        if calc_used { "yes" } else { "no" }
    );

    Ok(())
}

async fn test_todo_manager_streaming(
    client: &Client,
    tools: &std::collections::HashMap<String, Arc<dyn Tool>>,
    subsystem: &SkillSubsystem,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(8)
        .with_plugin(subsystem.plugin());

    let thread = Thread::with_initial_state("test-todo-skills", json!({ "counter": 0 }))
        .with_message(Message::system(
            "You are a helpful assistant with access to skills and tools. \
             When managing tasks, first activate the todo-manager skill, \
             then use its instructions and the counter tool.",
        ))
        .with_message(Message::user(
            "I want to manage my todo list. Activate the todo-manager skill, \
             then add 3 tasks: buy groceries, clean house, and exercise. \
             Use the counter to track the task count.",
        ));

    print!("User: Add 3 tasks to todo list.\nAssistant: ");

    let mut stream = run_loop_stream(
        client.clone(),
        config,
        thread,
        tools.clone(),
        RunContext::default(),
    );

    let mut final_text = String::new();
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::TextDelta { delta } => {
                print!("{delta}");
                std::io::Write::flush(&mut std::io::stdout())?;
                final_text.push_str(&delta);
            }
            AgentEvent::ToolCallStart { name, .. } => {
                print!("\n  [calling tool: {name}]");
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
                println!("\n[Error: {message}]");
            }
            _ => {}
        }
    }

    Ok(())
}

async fn test_multi_skill(
    client: &Client,
    tools: &std::collections::HashMap<String, Arc<dyn Tool>>,
    subsystem: &SkillSubsystem,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AgentConfig::new("deepseek-chat")
        .with_max_rounds(8)
        .with_plugin(subsystem.plugin());

    // Multi-turn conversation activating both skills
    let mut thread = Thread::with_initial_state("test-multi-skill", json!({ "counter": 0 }))
        .with_message(Message::system(
            "You are a helpful assistant with skills and tools. \
             Activate the appropriate skill before performing related tasks.",
        ));

    // Turn 1: Use unit converter
    thread = thread.with_message(Message::user(
        "I need to convert 5 pounds to kilograms. Use the unit-converter skill.",
    ));

    let (new_thread, response) = run_loop(client, &config, thread, tools)
        .await
        .map_err(|e| format!("LLM error: {e}"))?;
    thread = new_thread;

    println!("Turn 1 - User: Convert 5 pounds to kilograms.");
    println!("Turn 1 - Assistant: {response}");

    // Turn 2: Use todo manager
    thread = thread.with_message(Message::user(
        "Now activate the todo-manager skill and add a task: 'buy 2.27kg of flour'. \
         Use the counter to track tasks.",
    ));

    let (new_thread, response) = run_loop(client, &config, thread, tools)
        .await
        .map_err(|e| format!("LLM error: {e}"))?;
    thread = new_thread;

    println!("\nTurn 2 - User: Add a task using todo-manager skill.");
    println!("Turn 2 - Assistant: {response}");

    // Summary
    let final_state = thread.rebuild_state()?;
    println!("\nSession summary:");
    println!("  Total messages: {}", thread.message_count());
    println!("  Total patches: {}", thread.patch_count());
    println!(
        "  Final state: {}",
        serde_json::to_string_pretty(&final_state)?
    );

    // Check which skills were activated
    let skill_calls: Vec<_> = thread
        .messages
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .filter(|c| c.name == "skill")
        .collect();

    println!("  Skill activations: {}", skill_calls.len());
    for call in &skill_calls {
        println!("    - args: {}", call.arguments);
    }

    Ok(())
}
