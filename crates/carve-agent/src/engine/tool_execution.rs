//! Tool execution utilities.

use crate::contracts::conversation::ToolCall;
use crate::contracts::context::AgentState;
use crate::contracts::traits::tool::{Tool, ToolResult};
use carve_state::{ScopeState, TrackedPatch};
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
    execute_single_tool_with_scope(tool, call, state, None).await
}

/// Execute a single tool call with an optional scope context.
pub async fn execute_single_tool_with_scope(
    tool: Option<&dyn Tool>,
    call: &ToolCall,
    state: &Value,
    scope: Option<&ScopeState>,
) -> ToolExecution {
    let Some(tool) = tool else {
        return ToolExecution {
            call: call.clone(),
            result: ToolResult::error(&call.name, format!("Tool '{}' not found", call.name)),
            patch: None,
        };
    };

    // Create context for this tool call
    let ctx = AgentState::new(state, &call.id, format!("tool:{}", call.name)).with_scope(scope);

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
        let mut exec = execute_single_tool(tool.as_deref(), call, &state).await;

        // Apply patch to state for next tool
        if let Some(ref patch) = exec.patch {
            match apply_patch(&state, patch.patch()) {
                Ok(new_state) => {
                    state = new_state;
                }
                Err(e) => {
                    exec.result = ToolResult::error(
                        &call.name,
                        format!("failed to apply tool patch for call '{}': {}", call.id, e),
                    );
                    exec.patch = None;
                    executions.push(exec);
                    break;
                }
            }
        }

        executions.push(exec);
    }

    (state, executions)
}

/// Collect patches from executions.
pub fn collect_patches(executions: &[ToolExecution]) -> Vec<TrackedPatch> {
    executions.iter().filter_map(|e| e.patch.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::traits::tool::{ToolDescriptor, ToolError};
    use async_trait::async_trait;
    use carve_state_derive::State;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("echo", "Echo", "Echo the input")
        }

        async fn execute(&self, args: Value, _ctx: &AgentState<'_>) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("echo", args))
        }
    }

    struct CounterTool;

    #[async_trait]
    impl Tool for CounterTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("counter", "Counter", "Increment a counter")
        }

        async fn execute(&self, _args: Value, _ctx: &AgentState<'_>) -> Result<ToolResult, ToolError> {
            // In real usage, state would be accessed via ctx.state::<T>()
            // For this test, we just return success
            Ok(ToolResult::success("counter", json!({"incremented": true})))
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
    struct SequentialCounterState {
        counter: i64,
    }

    struct InvalidIncrementTool;

    #[async_trait]
    impl Tool for InvalidIncrementTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(
                "invalid_increment",
                "InvalidIncrement",
                "Produces an increment patch that can fail to apply",
            )
        }

        async fn execute(&self, _args: Value, ctx: &AgentState<'_>) -> Result<ToolResult, ToolError> {
            let state = ctx.state::<SequentialCounterState>("");
            state.increment_counter(1);
            Ok(ToolResult::success(
                "invalid_increment",
                json!({"ok": true}),
            ))
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
        tools.insert("counter".to_string(), Arc::new(CounterTool));

        let calls = vec![
            ToolCall::new("call_1", "echo", json!({"n": 1})),
            ToolCall::new("call_2", "echo", json!({"n": 2})),
            ToolCall::new("call_3", "counter", json!({})),
            ToolCall::new("call_4", "unknown", json!({})),
        ];

        let state = json!({});
        let executions = execute_tools_parallel(&tools, &calls, &state).await;

        assert_eq!(executions.len(), 4);
        assert!(executions[0].result.is_success());
        assert!(executions[1].result.is_success());
        assert!(executions[2].result.is_success());
        assert!(executions[3].result.is_error());
    }

    #[tokio::test]
    async fn test_collect_patches() {
        use carve_state::{path, Op, Patch};

        let executions = vec![
            ToolExecution {
                call: ToolCall::new("1", "a", json!({})),
                result: ToolResult::success("a", json!({})),
                patch: Some(TrackedPatch::new(
                    Patch::new().with_op(Op::set(path!("a"), json!(1))),
                )),
            },
            ToolExecution {
                call: ToolCall::new("2", "b", json!({})),
                result: ToolResult::success("b", json!({})),
                patch: None,
            },
            ToolExecution {
                call: ToolCall::new("3", "c", json!({})),
                result: ToolResult::success("c", json!({})),
                patch: Some(TrackedPatch::new(
                    Patch::new().with_op(Op::set(path!("c"), json!(3))),
                )),
            },
        ];

        let patches = collect_patches(&executions);
        assert_eq!(patches.len(), 2);
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

        let (final_state, executions) = execute_tools_sequential(&tools, &calls, &state).await;

        assert_eq!(executions.len(), 2);
        assert!(executions[0].result.is_success());
        assert!(executions[1].result.is_success());
        assert!(final_state.is_object());
    }

    #[tokio::test]
    async fn test_execute_tools_sequential_surfaces_patch_apply_failure() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(
            "invalid_increment".to_string(),
            Arc::new(InvalidIncrementTool),
        );
        tools.insert("echo".to_string(), Arc::new(EchoTool));

        let calls = vec![
            ToolCall::new("call_bad", "invalid_increment", json!({})),
            ToolCall::new("call_echo", "echo", json!({"n": 2})),
        ];

        // Missing `counter` makes increment patch application fail.
        let state = json!({});
        let (_final_state, executions) = execute_tools_sequential(&tools, &calls, &state).await;

        assert_eq!(
            executions.len(),
            1,
            "execution should stop once patch apply fails"
        );
        assert!(
            executions[0].result.is_error(),
            "patch apply failure should surface as tool execution error"
        );
        assert!(
            executions[0]
                .result
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("failed to apply tool patch"),
            "expected patch apply error message, got: {:?}",
            executions[0].result.message
        );
    }

    #[tokio::test]
    async fn test_tool_execution_error() {
        struct FailingTool;

        #[async_trait]
        impl Tool for FailingTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("failing", "Failing", "Always fails")
            }

            async fn execute(
                &self,
                _args: Value,
                _ctx: &AgentState<'_>,
            ) -> Result<ToolResult, ToolError> {
                Err(ToolError::ExecutionFailed(
                    "Intentional failure".to_string(),
                ))
            }
        }

        let tool = FailingTool;
        let call = ToolCall::new("call_1", "failing", json!({}));
        let state = json!({});

        let exec = execute_single_tool(Some(&tool), &call, &state).await;

        assert!(exec.result.is_error());
        assert!(exec
            .result
            .message
            .as_ref()
            .unwrap()
            .contains("Intentional failure"));
    }

    #[tokio::test]
    async fn test_execute_single_tool_with_scope_reads() {
        use carve_state::ScopeState;

        /// Tool that reads user_id from scope and returns it.
        struct ScopeReaderTool;

        #[async_trait]
        impl Tool for ScopeReaderTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("scope_reader", "ScopeReader", "Reads scope values")
            }

            async fn execute(
                &self,
                _args: Value,
                ctx: &AgentState<'_>,
            ) -> Result<ToolResult, ToolError> {
                let user_id = ctx
                    .scope_value("user_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                Ok(ToolResult::success(
                    "scope_reader",
                    json!({"user_id": user_id}),
                ))
            }
        }

        let mut scope = ScopeState::new();
        scope.set("user_id", "u-42").unwrap();

        let tool = ScopeReaderTool;
        let call = ToolCall::new("call_1", "scope_reader", json!({}));
        let state = json!({});

        let exec = execute_single_tool_with_scope(Some(&tool), &call, &state, Some(&scope)).await;

        assert!(exec.result.is_success());
        assert_eq!(exec.result.data["user_id"], "u-42");
    }

    #[tokio::test]
    async fn test_execute_single_tool_with_scope_none() {
        /// Tool that checks scope_ref is None.
        struct ScopeCheckerTool;

        #[async_trait]
        impl Tool for ScopeCheckerTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("scope_checker", "ScopeChecker", "Checks scope presence")
            }

            async fn execute(
                &self,
                _args: Value,
                ctx: &AgentState<'_>,
            ) -> Result<ToolResult, ToolError> {
                let has_scope = ctx.scope_ref().is_some();
                Ok(ToolResult::success(
                    "scope_checker",
                    json!({"has_scope": has_scope}),
                ))
            }
        }

        let tool = ScopeCheckerTool;
        let call = ToolCall::new("call_1", "scope_checker", json!({}));
        let state = json!({});

        // Without scope
        let exec = execute_single_tool_with_scope(Some(&tool), &call, &state, None).await;
        assert_eq!(exec.result.data["has_scope"], false);

        // With scope
        let scope = ScopeState::new();
        let exec = execute_single_tool_with_scope(Some(&tool), &call, &state, Some(&scope)).await;
        assert_eq!(exec.result.data["has_scope"], true);
    }

    #[tokio::test]
    async fn test_execute_with_scope_sensitive_key() {
        use carve_state::ScopeState;

        /// Tool that reads a sensitive key from scope.
        struct SensitiveReaderTool;

        #[async_trait]
        impl Tool for SensitiveReaderTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("sensitive", "Sensitive", "Reads sensitive key")
            }

            async fn execute(
                &self,
                _args: Value,
                ctx: &AgentState<'_>,
            ) -> Result<ToolResult, ToolError> {
                let scope = ctx.scope_ref().unwrap();
                let token = scope.value("token").and_then(|v| v.as_str()).unwrap();
                let is_sensitive = scope.is_sensitive("token");
                Ok(ToolResult::success(
                    "sensitive",
                    json!({"token_len": token.len(), "is_sensitive": is_sensitive}),
                ))
            }
        }

        let mut scope = ScopeState::new();
        scope.set_sensitive("token", "super-secret-token").unwrap();

        let tool = SensitiveReaderTool;
        let call = ToolCall::new("call_1", "sensitive", json!({}));
        let state = json!({});

        let exec =
            execute_single_tool_with_scope(Some(&tool), &call, &state, Some(&scope)).await;

        assert!(exec.result.is_success());
        assert_eq!(exec.result.data["token_len"], 18);
        assert_eq!(exec.result.data["is_sensitive"], true);
    }
}
