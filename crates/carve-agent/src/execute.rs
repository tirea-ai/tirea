//! Tool execution utilities.

use crate::plugin::AgentPlugin;
use crate::traits::tool::{Tool, ToolResult};
use crate::types::ToolCall;
use carve_state::{Context, TrackedPatch};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Result of executing a single tool.
#[derive(Debug, Clone)]
pub struct ToolExecution {
    /// The tool call that was executed.
    pub call: ToolCall,
    /// The result of execution.
    pub result: ToolResult,
    /// State changes from the tool (if any).
    pub patch: Option<TrackedPatch>,
}

/// Execute a single tool call.
///
/// This function:
/// 1. Creates a Context from the state snapshot
/// 2. Executes the tool
/// 3. Extracts any state changes as a TrackedPatch
///
/// # Arguments
///
/// * `tool` - The tool to execute (or None if not found)
/// * `call` - The tool call with id, name, and arguments
/// * `state` - The current state snapshot (read-only)
pub async fn execute_single_tool(
    tool: Option<&dyn Tool>,
    call: &ToolCall,
    state: &Value,
) -> ToolExecution {
    let Some(tool) = tool else {
        return ToolExecution {
            call: call.clone(),
            result: ToolResult::error(&call.name, format!("Tool '{}' not found", call.name)),
            patch: None,
        };
    };

    // Create context for this tool call
    let ctx = Context::new(state, &call.id, format!("tool:{}", call.name));

    // Execute the tool
    let result = match tool.execute(call.arguments.clone(), &ctx).await {
        Ok(r) => r,
        Err(e) => ToolResult::error(&call.name, e.to_string()),
    };

    // Extract any state changes
    let patch = ctx.take_patch();
    let patch = if patch.patch().is_empty() {
        None
    } else {
        Some(patch)
    };

    ToolExecution {
        call: call.clone(),
        result,
        patch,
    }
}

/// Execute a single tool call with plugin hooks.
///
/// This function runs plugin hooks before and after tool execution:
/// 1. Create Context from state
/// 2. Run `before_tool_execute` hooks on all plugins
/// 3. Check if blocked/pending - if so, skip tool execution
/// 4. Execute the tool
/// 5. Run `after_tool_execute` hooks on all plugins
/// 6. Combine patches from hooks and tool
pub async fn execute_single_tool_with_plugins(
    tool: Option<&dyn Tool>,
    call: &ToolCall,
    state: &Value,
    plugins: &[Arc<dyn AgentPlugin>],
) -> ToolExecution {
    let Some(tool) = tool else {
        return ToolExecution {
            call: call.clone(),
            result: ToolResult::error(&call.name, format!("Tool '{}' not found", call.name)),
            patch: None,
        };
    };

    // Create context for this tool call
    let ctx = Context::new(state, &call.id, format!("tool:{}", call.name));

    // Run before_tool_execute hooks
    for plugin in plugins {
        plugin
            .before_tool_execute(&ctx, &call.name, &call.arguments)
            .await;
    }

    // Check if blocked by plugins
    if ctx.is_blocked() {
        let reason = ctx.block_reason().unwrap_or_else(|| "Blocked by plugin".to_string());
        let patch = ctx.take_patch();
        return ToolExecution {
            call: call.clone(),
            result: ToolResult::error(&call.name, reason),
            patch: if patch.patch().is_empty() {
                None
            } else {
                Some(patch)
            },
        };
    }

    // Check if pending user interaction
    if ctx.is_pending() {
        let patch = ctx.take_patch();
        return ToolExecution {
            call: call.clone(),
            result: ToolResult::pending(&call.name, "Waiting for user confirmation"),
            patch: if patch.patch().is_empty() {
                None
            } else {
                Some(patch)
            },
        };
    }

    // Execute the tool
    let result = match tool.execute(call.arguments.clone(), &ctx).await {
        Ok(r) => r,
        Err(e) => ToolResult::error(&call.name, e.to_string()),
    };

    // Run after_tool_execute hooks
    for plugin in plugins {
        plugin.after_tool_execute(&ctx, &call.name, &result).await;
    }

    // Extract any state changes (includes both plugin and tool patches)
    let patch = ctx.take_patch();
    let patch = if patch.patch().is_empty() {
        None
    } else {
        Some(patch)
    };

    ToolExecution {
        call: call.clone(),
        result,
        patch,
    }
}

/// Execute multiple tool calls in parallel with plugin hooks.
pub async fn execute_tools_parallel_with_plugins(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[ToolCall],
    state: &Value,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Vec<ToolExecution> {
    use futures::future::join_all;

    let futures = calls.iter().map(|call| {
        let tool = tools.get(&call.name).cloned();
        let state = state.clone();
        let call = call.clone();
        let plugins = plugins.to_vec();

        async move {
            execute_single_tool_with_plugins(tool.as_deref(), &call, &state, &plugins).await
        }
    });

    join_all(futures).await
}

/// Execute tool calls sequentially with plugin hooks.
pub async fn execute_tools_sequential_with_plugins(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[ToolCall],
    initial_state: &Value,
    plugins: &[Arc<dyn AgentPlugin>],
) -> (Value, Vec<ToolExecution>) {
    use carve_state::apply_patch;

    let mut state = initial_state.clone();
    let mut executions = Vec::with_capacity(calls.len());

    for call in calls {
        let tool = tools.get(&call.name).cloned();
        let exec =
            execute_single_tool_with_plugins(tool.as_deref(), call, &state, plugins).await;

        // Apply patch to state for next tool
        if let Some(ref patch) = exec.patch {
            if let Ok(new_state) = apply_patch(&state, patch.patch()) {
                state = new_state;
            }
        }

        executions.push(exec);
    }

    (state, executions)
}

/// Execute multiple tool calls in parallel.
///
/// All tools receive the same state snapshot (they don't see each other's changes).
/// The patches should be applied in order after all tools complete.
///
/// # Arguments
///
/// * `tools` - Map of tool name to tool implementation
/// * `calls` - The tool calls to execute
/// * `state` - The current state snapshot (shared by all tools)
///
/// # Returns
///
/// Vector of execution results in the same order as the input calls.
pub async fn execute_tools_parallel(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[ToolCall],
    state: &Value,
) -> Vec<ToolExecution> {
    use futures::future::join_all;

    let futures = calls.iter().map(|call| {
        let tool = tools.get(&call.name).cloned();
        let state = state.clone();
        let call = call.clone();

        async move { execute_single_tool(tool.as_deref(), &call, &state).await }
    });

    join_all(futures).await
}

/// Execute tool calls sequentially.
///
/// Each tool sees the state changes from previous tools.
///
/// # Arguments
///
/// * `tools` - Map of tool name to tool implementation
/// * `calls` - The tool calls to execute
/// * `initial_state` - The initial state
///
/// # Returns
///
/// Tuple of (final_state, executions).
pub async fn execute_tools_sequential(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[ToolCall],
    initial_state: &Value,
) -> (Value, Vec<ToolExecution>) {
    use carve_state::apply_patch;

    let mut state = initial_state.clone();
    let mut executions = Vec::with_capacity(calls.len());

    for call in calls {
        let tool = tools.get(&call.name).cloned();
        let exec = execute_single_tool(tool.as_deref(), call, &state).await;

        // Apply patch to state for next tool
        if let Some(ref patch) = exec.patch {
            if let Ok(new_state) = apply_patch(&state, patch.patch()) {
                state = new_state;
            }
        }

        executions.push(exec);
    }

    (state, executions)
}

/// Collect patches from executions.
pub fn collect_patches(executions: &[ToolExecution]) -> Vec<TrackedPatch> {
    executions
        .iter()
        .filter_map(|e| e.patch.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::tool::{ToolDescriptor, ToolError};
    use async_trait::async_trait;
    use serde_json::json;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("echo", "Echo", "Echo the input")
        }

        async fn execute(
            &self,
            args: Value,
            _ctx: &Context<'_>,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("echo", args))
        }
    }

    struct CounterTool;

    #[async_trait]
    impl Tool for CounterTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("counter", "Counter", "Increment a counter")
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &Context<'_>,
        ) -> Result<ToolResult, ToolError> {
            // In real usage, state would be accessed via ctx.state::<T>()
            // For this test, we just return success
            Ok(ToolResult::success("counter", json!({"incremented": true})))
        }
    }

    #[tokio::test]
    async fn test_execute_single_tool_not_found() {
        let call = ToolCall::new("call_1", "nonexistent", json!({}));
        let state = json!({});

        let exec = execute_single_tool(None, &call, &state).await;

        assert!(exec.result.is_error());
        assert!(exec.patch.is_none());
    }

    #[tokio::test]
    async fn test_execute_single_tool_success() {
        let tool = EchoTool;
        let call = ToolCall::new("call_1", "echo", json!({"msg": "hello"}));
        let state = json!({});

        let exec = execute_single_tool(Some(&tool), &call, &state).await;

        assert!(exec.result.is_success());
        assert_eq!(exec.result.data["msg"], "hello");
    }

    #[tokio::test]
    async fn test_execute_tools_parallel() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool));

        let calls = vec![
            ToolCall::new("call_1", "echo", json!({"n": 1})),
            ToolCall::new("call_2", "echo", json!({"n": 2})),
            ToolCall::new("call_3", "unknown", json!({})),
        ];

        let state = json!({});
        let executions = execute_tools_parallel(&tools, &calls, &state).await;

        assert_eq!(executions.len(), 3);
        assert!(executions[0].result.is_success());
        assert!(executions[1].result.is_success());
        assert!(executions[2].result.is_error());
    }

    #[tokio::test]
    async fn test_collect_patches() {
        use carve_state::{path, Op, Patch};

        let executions = vec![
            ToolExecution {
                call: ToolCall::new("1", "a", json!({})),
                result: ToolResult::success("a", json!({})),
                patch: Some(TrackedPatch::new(Patch::new().with_op(Op::set(path!("a"), json!(1))))),
            },
            ToolExecution {
                call: ToolCall::new("2", "b", json!({})),
                result: ToolResult::success("b", json!({})),
                patch: None,
            },
            ToolExecution {
                call: ToolCall::new("3", "c", json!({})),
                result: ToolResult::success("c", json!({})),
                patch: Some(TrackedPatch::new(Patch::new().with_op(Op::set(path!("c"), json!(3))))),
            },
        ];

        let patches = collect_patches(&executions);
        assert_eq!(patches.len(), 2);
    }

    // ============================================================================
    // Plugin Execution Tests
    // ============================================================================

    #[tokio::test]
    async fn test_execute_single_tool_with_plugins_tool_not_found() {
        let call = ToolCall::new("call_1", "nonexistent", json!({}));
        let state = json!({});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![];

        let exec = execute_single_tool_with_plugins(None, &call, &state, &plugins).await;

        assert!(exec.result.is_error());
        assert!(exec.result.message.as_ref().unwrap().contains("not found"));
        assert!(exec.patch.is_none());
    }

    #[tokio::test]
    async fn test_execute_single_tool_with_plugins_success() {
        let tool = EchoTool;
        let call = ToolCall::new("call_1", "echo", json!({"msg": "hello"}));
        let state = json!({});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![];

        let exec = execute_single_tool_with_plugins(Some(&tool), &call, &state, &plugins).await;

        assert!(exec.result.is_success());
        assert_eq!(exec.result.data["msg"], "hello");
    }

    #[tokio::test]
    async fn test_execute_single_tool_with_blocking_plugin() {
        use crate::plugin::AgentPlugin;
        use async_trait::async_trait;

        struct BlockingPlugin;

        #[async_trait]
        impl AgentPlugin for BlockingPlugin {
            fn id(&self) -> &str { "blocker" }

            async fn before_tool_execute(&self, ctx: &Context<'_>, _tool_id: &str, _args: &Value) {
                ctx.set_blocked("Blocked by plugin");
            }
        }

        let tool = EchoTool;
        let call = ToolCall::new("call_1", "echo", json!({"msg": "hello"}));
        let state = json!({});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(BlockingPlugin)];

        let exec = execute_single_tool_with_plugins(Some(&tool), &call, &state, &plugins).await;

        // Tool should be blocked
        assert!(exec.result.is_error());
        assert!(exec.result.message.as_ref().unwrap().contains("Blocked"));
    }

    #[tokio::test]
    async fn test_execute_single_tool_with_pending_plugin() {
        use crate::plugin::AgentPlugin;
        use async_trait::async_trait;

        struct PendingPlugin;

        #[async_trait]
        impl AgentPlugin for PendingPlugin {
            fn id(&self) -> &str { "pending" }

            async fn before_tool_execute(&self, ctx: &Context<'_>, _tool_id: &str, _args: &Value) {
                ctx.set_pending(json!({"confirm": true}));
            }
        }

        let tool = EchoTool;
        let call = ToolCall::new("call_1", "echo", json!({"msg": "hello"}));
        let state = json!({});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(PendingPlugin)];

        let exec = execute_single_tool_with_plugins(Some(&tool), &call, &state, &plugins).await;

        // Tool should be pending
        assert!(exec.result.is_pending());
    }

    #[tokio::test]
    async fn test_execute_tools_parallel_with_plugins() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool));

        let calls = vec![
            ToolCall::new("call_1", "echo", json!({"n": 1})),
            ToolCall::new("call_2", "echo", json!({"n": 2})),
        ];

        let state = json!({});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![];

        let executions = execute_tools_parallel_with_plugins(&tools, &calls, &state, &plugins).await;

        assert_eq!(executions.len(), 2);
        assert!(executions[0].result.is_success());
        assert!(executions[1].result.is_success());
    }

    #[tokio::test]
    async fn test_execute_tools_sequential_with_plugins() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool));

        let calls = vec![
            ToolCall::new("call_1", "echo", json!({"n": 1})),
            ToolCall::new("call_2", "echo", json!({"n": 2})),
        ];

        let state = json!({});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![];

        let (final_state, executions) = execute_tools_sequential_with_plugins(&tools, &calls, &state, &plugins).await;

        assert_eq!(executions.len(), 2);
        assert!(executions[0].result.is_success());
        assert!(executions[1].result.is_success());
        // Final state should still be empty since EchoTool doesn't modify state
        assert!(final_state.is_object());
    }

    #[tokio::test]
    async fn test_execute_tools_sequential() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool));

        let calls = vec![
            ToolCall::new("call_1", "echo", json!({"n": 1})),
            ToolCall::new("call_2", "echo", json!({"n": 2})),
        ];

        let state = json!({});

        let (final_state, executions) = crate::execute::execute_tools_sequential(&tools, &calls, &state).await;

        assert_eq!(executions.len(), 2);
        assert!(executions[0].result.is_success());
        assert!(executions[1].result.is_success());
        assert!(final_state.is_object());
    }

    #[tokio::test]
    async fn test_execute_single_tool_with_after_hook() {
        use crate::plugin::AgentPlugin;
        use async_trait::async_trait;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct AfterHookPlugin {
            after_called: AtomicBool,
        }

        impl AfterHookPlugin {
            fn new() -> Self {
                Self { after_called: AtomicBool::new(false) }
            }
        }

        #[async_trait]
        impl AgentPlugin for AfterHookPlugin {
            fn id(&self) -> &str { "after_hook" }

            async fn after_tool_execute(&self, _ctx: &Context<'_>, _tool_id: &str, _result: &ToolResult) {
                self.after_called.store(true, Ordering::SeqCst);
            }
        }

        let tool = EchoTool;
        let call = ToolCall::new("call_1", "echo", json!({"msg": "hello"}));
        let state = json!({});
        let plugin = Arc::new(AfterHookPlugin::new());
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![plugin.clone()];

        let exec = execute_single_tool_with_plugins(Some(&tool), &call, &state, &plugins).await;

        assert!(exec.result.is_success());
        assert!(plugin.after_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_execute_tools_sequential_with_state_changes() {
        use carve_state::{path, Op, Patch};

        struct StatefulTool;

        #[async_trait]
        impl Tool for StatefulTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("stateful", "Stateful", "Modifies state")
            }

            async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
                // Create a patch via direct ops
                let ops = vec![Op::set(path!("counter"), json!(42))];
                for op in ops {
                    ctx.state::<crate::plugins::ContextDataState>("context").data_insert("modified".to_string(), json!(true));
                }
                Ok(ToolResult::success("stateful", json!({"done": true})))
            }
        }

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("stateful".to_string(), Arc::new(StatefulTool));

        let calls = vec![ToolCall::new("call_1", "stateful", json!({}))];
        let state = json!({"context": {"data": {}}});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![];

        let (final_state, executions) = execute_tools_sequential_with_plugins(&tools, &calls, &state, &plugins).await;

        assert_eq!(executions.len(), 1);
        assert!(executions[0].result.is_success());
        // State should have the modification
        assert!(final_state["context"]["data"]["modified"] == json!(true));
    }

    #[tokio::test]
    async fn test_block_with_patch() {
        // Test that blocking returns a patch if the plugin modified state before blocking
        use crate::plugins::{ContextDataExt, ExecutionContextExt};

        struct BlockWithPatchPlugin;

        #[async_trait]
        impl AgentPlugin for BlockWithPatchPlugin {
            fn id(&self) -> &str {
                "block_with_patch"
            }

            async fn before_tool_execute(&self, ctx: &Context<'_>, _tool_id: &str, _args: &Value) {
                // First modify state
                ctx.set("modified_before_block", json!(true));
                // Then block
                ctx.block("Blocked after state modification");
            }
        }

        let tool = EchoTool;
        let call = ToolCall::new("call_1", "echo", json!({"msg": "test"}));
        let state = json!({"context": {"data": {}}});
        let plugin = Arc::new(BlockWithPatchPlugin);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![plugin];

        let exec = execute_single_tool_with_plugins(Some(&tool), &call, &state, &plugins).await;

        assert!(exec.result.is_error());
        // Should have a patch from the state modification
        assert!(exec.patch.is_some());
    }

    #[tokio::test]
    async fn test_pending_with_patch() {
        // Test that pending returns a patch if the plugin modified state
        use crate::plugins::{ContextDataExt, ExecutionContextExt};
        use crate::state_types::Interaction;

        struct PendingWithPatchPlugin;

        #[async_trait]
        impl AgentPlugin for PendingWithPatchPlugin {
            fn id(&self) -> &str {
                "pending_with_patch"
            }

            async fn before_tool_execute(&self, ctx: &Context<'_>, _tool_id: &str, _args: &Value) {
                // First modify state
                ctx.set("modified_before_pending", json!(true));
                // Then set pending
                ctx.pending(Interaction::confirm("confirm_1", "Please confirm"));
            }
        }

        let tool = EchoTool;
        let call = ToolCall::new("call_1", "echo", json!({"msg": "test"}));
        let state = json!({"context": {"data": {}}});
        let plugin = Arc::new(PendingWithPatchPlugin);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![plugin];

        let exec = execute_single_tool_with_plugins(Some(&tool), &call, &state, &plugins).await;

        assert!(exec.result.is_pending());
        // Should have a patch from the state modification
        assert!(exec.patch.is_some());
    }

    #[tokio::test]
    async fn test_tool_execution_error() {
        // Test that tool execution errors are handled correctly
        struct FailingTool;

        #[async_trait]
        impl Tool for FailingTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("failing", "Failing", "Always fails")
            }

            async fn execute(&self, _args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
                Err(ToolError::ExecutionFailed("Intentional failure".to_string()))
            }
        }

        let tool = FailingTool;
        let call = ToolCall::new("call_1", "failing", json!({}));
        let state = json!({});
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![];

        let exec = execute_single_tool_with_plugins(Some(&tool), &call, &state, &plugins).await;

        assert!(exec.result.is_error());
        assert!(exec.result.message.as_ref().unwrap().contains("Intentional failure"));
    }
}
