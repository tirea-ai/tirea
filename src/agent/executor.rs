//! Tool execution strategies: Sequential and Parallel.

use std::collections::HashMap;
use std::sync::Arc;

use crate::contract::message::ToolCall;
use crate::contract::suspension::ToolCallOutcome;
use crate::contract::tool::{Tool, ToolResult};
use async_trait::async_trait;

/// Result of executing a single tool call.
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub call: ToolCall,
    pub result: ToolResult,
    pub outcome: ToolCallOutcome,
}

/// Error from tool execution strategy.
#[derive(Debug, thiserror::Error)]
pub enum ToolExecutorError {
    #[error("tool execution cancelled")]
    Cancelled,
    #[error("tool execution failed: {0}")]
    Failed(String),
}

/// Strategy abstraction for tool execution.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute tool calls and return results.
    async fn execute(
        &self,
        tools: &HashMap<String, Arc<dyn Tool>>,
        calls: &[ToolCall],
    ) -> Result<Vec<ToolExecutionResult>, ToolExecutorError>;

    /// Strategy name for logging.
    fn name(&self) -> &'static str;
}

/// Execute tool calls one by one. Each tool sees the results of previous tools.
/// Stops at first suspension.
#[derive(Debug, Clone, Copy, Default)]
pub struct SequentialToolExecutor;

#[async_trait]
impl ToolExecutor for SequentialToolExecutor {
    async fn execute(
        &self,
        tools: &HashMap<String, Arc<dyn Tool>>,
        calls: &[ToolCall],
    ) -> Result<Vec<ToolExecutionResult>, ToolExecutorError> {
        let mut results = Vec::with_capacity(calls.len());

        for call in calls {
            let result = execute_single_tool(tools, call).await;
            let outcome = ToolCallOutcome::from_tool_result(&result);

            results.push(ToolExecutionResult {
                call: call.clone(),
                result,
                outcome,
            });

            // Stop at first suspension
            if results
                .last()
                .is_some_and(|r| r.outcome == ToolCallOutcome::Suspended)
            {
                break;
            }
        }

        Ok(results)
    }

    fn name(&self) -> &'static str {
        "sequential"
    }
}

/// Policy controlling when resume decisions are replayed into tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionReplayPolicy {
    /// Replay each resolved suspended call as soon as its decision arrives.
    Immediate,
    /// Replay only when all currently suspended calls have decisions.
    BatchAllSuspended,
}

/// Parallel execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelMode {
    /// All concurrently, batch approval gate — wait for all decisions.
    BatchApproval,
    /// All concurrently, streaming results — replay decisions immediately.
    Streaming,
}

/// Execute all tool calls concurrently. All tools see the same frozen snapshot.
///
/// Two modes differ in how suspension decisions are replayed:
/// - `BatchApproval`: wait for all suspended calls to have decisions before replay
/// - `Streaming`: replay each decision immediately as it arrives
#[derive(Debug, Clone, Copy)]
pub struct ParallelToolExecutor {
    mode: ParallelMode,
}

impl ParallelToolExecutor {
    pub const fn batch_approval() -> Self {
        Self {
            mode: ParallelMode::BatchApproval,
        }
    }

    pub const fn streaming() -> Self {
        Self {
            mode: ParallelMode::Streaming,
        }
    }

    /// How the runtime should replay resolved suspend decisions.
    pub fn decision_replay_policy(&self) -> DecisionReplayPolicy {
        match self.mode {
            ParallelMode::BatchApproval => DecisionReplayPolicy::BatchAllSuspended,
            ParallelMode::Streaming => DecisionReplayPolicy::Immediate,
        }
    }

    /// Whether the runtime should enforce parallel patch conflict checks.
    pub fn requires_conflict_check(&self) -> bool {
        true
    }
}

impl Default for ParallelToolExecutor {
    fn default() -> Self {
        Self::streaming()
    }
}

#[async_trait]
impl ToolExecutor for ParallelToolExecutor {
    async fn execute(
        &self,
        tools: &HashMap<String, Arc<dyn Tool>>,
        calls: &[ToolCall],
    ) -> Result<Vec<ToolExecutionResult>, ToolExecutorError> {
        use futures::future::join_all;

        let futures: Vec<_> = calls
            .iter()
            .map(|call| {
                let tools = tools.clone();
                let call = call.clone();
                async move {
                    let result = execute_single_tool(&tools, &call).await;
                    let outcome = ToolCallOutcome::from_tool_result(&result);
                    ToolExecutionResult {
                        call,
                        result,
                        outcome,
                    }
                }
            })
            .collect();

        Ok(join_all(futures).await)
    }

    fn name(&self) -> &'static str {
        match self.mode {
            ParallelMode::BatchApproval => "parallel_batch_approval",
            ParallelMode::Streaming => "parallel_streaming",
        }
    }
}

/// Execute a single tool call, never panicking.
async fn execute_single_tool(
    tools: &HashMap<String, Arc<dyn Tool>>,
    call: &ToolCall,
) -> ToolResult {
    let Some(tool) = tools.get(&call.name) else {
        return ToolResult::error(&call.name, format!("tool '{}' not found", call.name));
    };

    if let Err(e) = tool.validate_args(&call.arguments) {
        return ToolResult::error(&call.name, e.to_string());
    }

    match tool.execute(call.arguments.clone()).await {
        Ok(result) => result,
        Err(e) => ToolResult::error(&call.name, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::tool::{ToolDescriptor, ToolError};
    use serde_json::{Value, json};

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("echo", "echo", "Echoes input")
        }

        async fn execute(&self, args: Value) -> Result<ToolResult, ToolError> {
            let msg = args
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("no message")
                .to_string();
            Ok(ToolResult::success_with_message("echo", args, msg))
        }
    }

    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("failing", "failing", "Always fails")
        }

        async fn execute(&self, _args: Value) -> Result<ToolResult, ToolError> {
            Err(ToolError::ExecutionFailed("intentional failure".into()))
        }
    }

    struct SuspendingTool;

    #[async_trait]
    impl Tool for SuspendingTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("suspending", "suspending", "Returns pending")
        }

        async fn execute(&self, _args: Value) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::suspended("suspending", "needs approval"))
        }
    }

    fn tool_map(tools: Vec<Arc<dyn Tool>>) -> HashMap<String, Arc<dyn Tool>> {
        tools.into_iter().map(|t| (t.descriptor().id, t)).collect()
    }

    // -- Sequential tests --

    #[tokio::test]
    async fn sequential_single_tool_success() {
        let tools = tool_map(vec![Arc::new(EchoTool)]);
        let calls = vec![ToolCall::new("c1", "echo", json!({"message": "hi"}))];
        let executor = SequentialToolExecutor;

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, ToolCallOutcome::Succeeded);
        assert!(results[0].result.is_success());
    }

    #[tokio::test]
    async fn sequential_partial_failure() {
        let tools = tool_map(vec![Arc::new(EchoTool), Arc::new(FailingTool)]);
        let calls = vec![
            ToolCall::new("c1", "echo", json!({"message": "ok"})),
            ToolCall::new("c2", "failing", json!({})),
        ];
        let executor = SequentialToolExecutor;

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].outcome, ToolCallOutcome::Succeeded);
        assert_eq!(results[1].outcome, ToolCallOutcome::Failed);
    }

    #[tokio::test]
    async fn sequential_stops_after_first_suspension() {
        let tools = tool_map(vec![Arc::new(SuspendingTool), Arc::new(EchoTool)]);
        let calls = vec![
            ToolCall::new("c1", "suspending", json!({})),
            ToolCall::new("c2", "echo", json!({"message": "should not run"})),
        ];
        let executor = SequentialToolExecutor;

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 1, "should stop after suspended tool");
        assert_eq!(results[0].outcome, ToolCallOutcome::Suspended);
    }

    #[tokio::test]
    async fn sequential_unknown_tool_returns_error() {
        let tools = tool_map(vec![]);
        let calls = vec![ToolCall::new("c1", "nonexistent", json!({}))];
        let executor = SequentialToolExecutor;

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, ToolCallOutcome::Failed);
        assert!(results[0].result.is_error());
    }

    #[tokio::test]
    async fn sequential_empty_calls() {
        let tools = tool_map(vec![Arc::new(EchoTool)]);
        let executor = SequentialToolExecutor;

        let results = executor.execute(&tools, &[]).await.unwrap();
        assert!(results.is_empty());
    }

    // -- Parallel tests --

    #[tokio::test]
    async fn parallel_all_succeed() {
        let tools = tool_map(vec![Arc::new(EchoTool)]);
        let calls = vec![
            ToolCall::new("c1", "echo", json!({"message": "first"})),
            ToolCall::new("c2", "echo", json!({"message": "second"})),
        ];
        let executor = ParallelToolExecutor::streaming();

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|r| r.outcome == ToolCallOutcome::Succeeded)
        );
    }

    #[tokio::test]
    async fn parallel_partial_failure() {
        let tools = tool_map(vec![Arc::new(EchoTool), Arc::new(FailingTool)]);
        let calls = vec![
            ToolCall::new("c1", "echo", json!({"message": "ok"})),
            ToolCall::new("c2", "failing", json!({})),
        ];
        let executor = ParallelToolExecutor::streaming();

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 2);
        let successes = results
            .iter()
            .filter(|r| r.outcome == ToolCallOutcome::Succeeded)
            .count();
        let failures = results
            .iter()
            .filter(|r| r.outcome == ToolCallOutcome::Failed)
            .count();
        assert_eq!(successes, 1);
        assert_eq!(failures, 1);
    }

    #[tokio::test]
    async fn parallel_does_not_stop_on_suspension() {
        let tools = tool_map(vec![Arc::new(SuspendingTool), Arc::new(EchoTool)]);
        let calls = vec![
            ToolCall::new("c1", "suspending", json!({})),
            ToolCall::new("c2", "echo", json!({"message": "runs anyway"})),
        ];
        let executor = ParallelToolExecutor::streaming();

        let results = executor.execute(&tools, &calls).await.unwrap();
        // Parallel executes ALL tools regardless of suspension
        assert_eq!(results.len(), 2);
        let suspended = results
            .iter()
            .filter(|r| r.outcome == ToolCallOutcome::Suspended)
            .count();
        let succeeded = results
            .iter()
            .filter(|r| r.outcome == ToolCallOutcome::Succeeded)
            .count();
        assert_eq!(suspended, 1);
        assert_eq!(succeeded, 1);
    }

    #[tokio::test]
    async fn parallel_empty_calls() {
        let tools = tool_map(vec![Arc::new(EchoTool)]);
        let executor = ParallelToolExecutor::streaming();

        let results = executor.execute(&tools, &[]).await.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn executor_names() {
        assert_eq!(SequentialToolExecutor.name(), "sequential");
        assert_eq!(
            ParallelToolExecutor::streaming().name(),
            "parallel_streaming"
        );
        assert_eq!(
            ParallelToolExecutor::batch_approval().name(),
            "parallel_batch_approval"
        );
    }

    #[test]
    fn parallel_default_is_streaming() {
        let executor = ParallelToolExecutor::default();
        assert_eq!(executor.name(), "parallel_streaming");
        assert_eq!(
            executor.decision_replay_policy(),
            DecisionReplayPolicy::Immediate
        );
    }

    #[test]
    fn parallel_batch_approval_policy() {
        let executor = ParallelToolExecutor::batch_approval();
        assert_eq!(
            executor.decision_replay_policy(),
            DecisionReplayPolicy::BatchAllSuspended
        );
        assert!(executor.requires_conflict_check());
    }

    #[test]
    fn parallel_streaming_policy() {
        let executor = ParallelToolExecutor::streaming();
        assert_eq!(
            executor.decision_replay_policy(),
            DecisionReplayPolicy::Immediate
        );
        assert!(executor.requires_conflict_check());
    }

    #[tokio::test]
    async fn batch_approval_executes_all_concurrently() {
        let tools = tool_map(vec![Arc::new(EchoTool)]);
        let calls = vec![
            ToolCall::new("c1", "echo", json!({"message": "a"})),
            ToolCall::new("c2", "echo", json!({"message": "b"})),
        ];
        let executor = ParallelToolExecutor::batch_approval();

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|r| r.outcome == ToolCallOutcome::Succeeded)
        );
    }

    #[tokio::test]
    async fn batch_approval_does_not_stop_on_suspension() {
        let tools = tool_map(vec![Arc::new(SuspendingTool), Arc::new(EchoTool)]);
        let calls = vec![
            ToolCall::new("c1", "suspending", json!({})),
            ToolCall::new("c2", "echo", json!({"message": "runs anyway"})),
        ];
        let executor = ParallelToolExecutor::batch_approval();

        let results = executor.execute(&tools, &calls).await.unwrap();
        assert_eq!(results.len(), 2);
    }
}
