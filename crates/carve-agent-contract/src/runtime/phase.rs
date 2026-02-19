//! Phase-based plugin execution system.
//!
//! This module provides the core types for the plugin phase system:
//! - `Phase`: Execution phases in the agent loop
//! - `StepContext`: Mutable context passed through all phases
//! - `ToolContext`: Tool-call state carried by `StepContext`

use crate::context::ToolCallContext;
use crate::runtime::interaction::Interaction;
use crate::runtime::result::StreamResult;
use crate::state::{Message, ToolCall};
use crate::tool::{ToolDescriptor, ToolResult};
use carve_state::{CarveResult, ScopeState, State, TrackedPatch};
use serde_json::Value;
use std::sync::Arc;

/// Execution phase in the agent loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// AgentState started (called once).
    RunStart,
    /// Step started - prepare context.
    StepStart,
    /// Before LLM inference - build messages, filter tools.
    BeforeInference,
    /// After LLM inference - process response.
    AfterInference,
    /// Before tool execution.
    BeforeToolExecute,
    /// After tool execution.
    AfterToolExecute,
    /// Step ended.
    StepEnd,
    /// AgentState ended (called once).
    RunEnd,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Phase::RunStart => write!(f, "RunStart"),
            Phase::StepStart => write!(f, "StepStart"),
            Phase::BeforeInference => write!(f, "BeforeInference"),
            Phase::AfterInference => write!(f, "AfterInference"),
            Phase::BeforeToolExecute => write!(f, "BeforeToolExecute"),
            Phase::AfterToolExecute => write!(f, "AfterToolExecute"),
            Phase::StepEnd => write!(f, "StepEnd"),
            Phase::RunEnd => write!(f, "RunEnd"),
        }
    }
}

/// Mutation policy enforced for each phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhasePolicy {
    /// Whether tool filtering (`StepContext::tools`) can be mutated.
    pub allow_tool_filter_mutation: bool,
    /// Whether `StepContext::skip_inference` can be mutated.
    pub allow_skip_inference_mutation: bool,
    /// Whether tool execution gate (`blocked/pending`) can be mutated.
    pub allow_tool_gate_mutation: bool,
}

impl PhasePolicy {
    pub const fn read_only() -> Self {
        Self {
            allow_tool_filter_mutation: false,
            allow_skip_inference_mutation: false,
            allow_tool_gate_mutation: false,
        }
    }
}

impl Phase {
    /// Return mutation policy for this phase.
    pub const fn policy(self) -> PhasePolicy {
        match self {
            Self::BeforeInference => PhasePolicy {
                allow_tool_filter_mutation: true,
                allow_skip_inference_mutation: true,
                allow_tool_gate_mutation: false,
            },
            Self::BeforeToolExecute => PhasePolicy {
                allow_tool_filter_mutation: false,
                allow_skip_inference_mutation: false,
                allow_tool_gate_mutation: true,
            },
            Self::RunStart
            | Self::StepStart
            | Self::AfterInference
            | Self::AfterToolExecute
            | Self::StepEnd
            | Self::RunEnd => PhasePolicy::read_only(),
        }
    }
}

/// Result of a step execution.
#[derive(Debug, Clone, PartialEq)]
pub enum StepOutcome {
    /// Continue to next step.
    Continue,
    /// AgentState complete.
    Complete,
    /// Pending user interaction.
    Pending(Interaction),
}

/// Context for the currently executing tool.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Tool call ID.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Tool arguments.
    pub args: Value,
    /// Tool execution result (set after execution).
    pub result: Option<ToolResult>,
    /// Whether execution is blocked.
    pub blocked: bool,
    /// Block reason.
    pub block_reason: Option<String>,
    /// Whether execution is pending user confirmation.
    pub pending: bool,
    /// Pending interaction.
    pub pending_interaction: Option<Interaction>,
}

impl ToolContext {
    /// Create a new tool context from a tool call.
    pub fn new(call: &ToolCall) -> Self {
        Self {
            id: call.id.clone(),
            name: call.name.clone(),
            args: call.arguments.clone(),
            result: None,
            blocked: false,
            block_reason: None,
            pending: false,
            pending_interaction: None,
        }
    }

    /// Check if the tool execution is blocked.
    pub fn is_blocked(&self) -> bool {
        self.blocked
    }

    /// Check if the tool execution is pending.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Stable idempotency key for this tool invocation.
    ///
    /// This is the same value as `tool_call_id`.
    pub fn idempotency_key(&self) -> &str {
        &self.id
    }
}

/// Step context - mutable state passed through all phases.
///
/// This is the primary interface for plugins to interact with the agent loop.
/// It provides access to session state, message building, tool filtering,
/// and flow control.
pub struct StepContext<'a> {
    // === Execution Context ===
    /// Execution context providing state access, scope, identity.
    ctx: ToolCallContext<'a>,

    // === Identity (from persistent entity) ===
    /// Thread id (read-only).
    thread_id: &'a str,

    // === Thread Messages (read-only snapshot from persistent entity) ===
    /// Messages from the thread (for building LLM requests, finding pending calls).
    messages: &'a [Arc<Message>],

    // === Message Building ===
    /// System context to append to system prompt [Position 1].
    pub system_context: Vec<String>,
    /// Session context messages (before user messages) [Position 2].
    pub session_context: Vec<String>,
    /// System reminders (after tool results) [Position 7].
    pub system_reminders: Vec<String>,

    // === Tool Control ===
    /// Available tool descriptors (can be filtered).
    pub tools: Vec<ToolDescriptor>,
    /// Current tool context (only valid during tool phases).
    pub tool: Option<ToolContext>,

    // === LLM Response ===
    /// LLM response (set after inference).
    pub response: Option<StreamResult>,

    // === Flow Control ===
    /// Skip LLM inference.
    pub skip_inference: bool,

    // === Pending State Changes ===
    /// Patches to apply to session state after this phase completes.
    pub pending_patches: Vec<TrackedPatch>,
}

impl<'a> StepContext<'a> {
    /// Create a new step context.
    pub fn new(
        ctx: ToolCallContext<'a>,
        thread_id: &'a str,
        messages: &'a [Arc<Message>],
        tools: Vec<ToolDescriptor>,
    ) -> Self {
        Self {
            ctx,
            thread_id,
            messages,
            system_context: Vec::new(),
            session_context: Vec::new(),
            system_reminders: Vec::new(),
            tools,
            tool: None,
            response: None,
            skip_inference: false,
            pending_patches: Vec::new(),
        }
    }

    // =========================================================================
    // Execution context access
    // =========================================================================

    /// Borrow the underlying execution context.
    pub fn ctx(&self) -> &ToolCallContext<'a> {
        &self.ctx
    }

    /// Thread id.
    pub fn thread_id(&self) -> &str {
        self.thread_id
    }

    /// Thread messages (read-only snapshot from persistent entity).
    pub fn messages(&self) -> &[Arc<Message>] {
        self.messages
    }

    /// Typed state reference at the type's canonical path.
    pub fn state_of<T: State>(&self) -> T::Ref<'_> {
        self.ctx.state_of::<T>()
    }

    /// Typed state reference at path.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        self.ctx.state::<T>(path)
    }

    /// Typed state reference writing to overlay (not persisted).
    pub fn override_state_of<T: State>(&self) -> T::Ref<'_> {
        self.ctx.override_state_of::<T>()
    }

    /// Borrow the scope state.
    pub fn scope(&self) -> &ScopeState {
        self.ctx.scope()
    }

    /// Typed scope state accessor.
    pub fn scope_state<T: State>(&self) -> CarveResult<T::Ref<'_>> {
        self.ctx.scope_state::<T>()
    }

    /// Read a scope value by key.
    pub fn scope_value(&self, key: &str) -> Option<&Value> {
        self.ctx.scope_value(key)
    }

    /// Snapshot the current document state.
    pub fn snapshot(&self) -> Value {
        self.ctx.snapshot()
    }

    /// Reset step-specific state for a new step.
    pub fn reset(&mut self) {
        self.system_context.clear();
        self.session_context.clear();
        self.system_reminders.clear();
        self.tool = None;
        self.response = None;
        self.skip_inference = false;
        self.pending_patches.clear();
    }

    // =========================================================================
    // Context Injection [Position 1, 2, 7]
    // =========================================================================

    /// Add system context (appended to system prompt) [Position 1].
    pub fn system(&mut self, content: impl Into<String>) {
        self.system_context.push(content.into());
    }

    /// Set system context (replaces existing) [Position 1].
    pub fn set_system(&mut self, content: impl Into<String>) {
        self.system_context = vec![content.into()];
    }

    /// Clear system context [Position 1].
    pub fn clear_system(&mut self) {
        self.system_context.clear();
    }

    /// Add session context message (before user messages) [Position 2].
    pub fn thread(&mut self, content: impl Into<String>) {
        self.session_context.push(content.into());
    }

    /// Set session context (replaces existing) [Position 2].
    pub fn set_session(&mut self, content: impl Into<String>) {
        self.session_context = vec![content.into()];
    }

    /// Clear session context [Position 2].
    pub fn clear_session(&mut self) {
        self.session_context.clear();
    }

    /// Add system reminder (after tool result) [Position 7].
    pub fn reminder(&mut self, content: impl Into<String>) {
        self.system_reminders.push(content.into());
    }

    /// Clear system reminders [Position 7].
    pub fn clear_reminders(&mut self) {
        self.system_reminders.clear();
    }

    // =========================================================================
    // Tool Filtering
    // =========================================================================

    /// Exclude a tool by ID.
    pub fn exclude(&mut self, tool_id: &str) {
        self.tools.retain(|t| t.id != tool_id);
    }

    /// Include only specified tools.
    pub fn include_only(&mut self, tool_ids: &[&str]) {
        self.tools.retain(|t| tool_ids.contains(&t.id.as_str()));
    }

    // =========================================================================
    // Tool Control (only valid during tool phases)
    // =========================================================================

    /// Get the current tool name (e.g., `"read_file"`).
    pub fn tool_name(&self) -> Option<&str> {
        self.tool.as_ref().map(|t| t.name.as_str())
    }

    /// Get the current tool call ID (e.g., `"call_abc123"`).
    pub fn tool_call_id(&self) -> Option<&str> {
        self.tool.as_ref().map(|t| t.id.as_str())
    }

    /// Get the current tool idempotency key.
    ///
    /// This is an alias of `tool_call_id`.
    pub fn tool_idempotency_key(&self) -> Option<&str> {
        self.tool_call_id()
    }

    /// Get current tool arguments.
    pub fn tool_args(&self) -> Option<&Value> {
        self.tool.as_ref().map(|t| &t.args)
    }

    /// Get current tool result.
    pub fn tool_result(&self) -> Option<&ToolResult> {
        self.tool.as_ref().and_then(|t| t.result.as_ref())
    }

    /// Check if current tool is blocked.
    pub fn tool_blocked(&self) -> bool {
        self.tool.as_ref().map(|t| t.blocked).unwrap_or(false)
    }

    /// Check if current tool is pending.
    pub fn tool_pending(&self) -> bool {
        self.tool.as_ref().map(|t| t.pending).unwrap_or(false)
    }

    /// Block current tool execution.
    pub fn block(&mut self, reason: impl Into<String>) {
        if let Some(ref mut tool) = self.tool {
            tool.blocked = true;
            tool.block_reason = Some(reason.into());
        }
    }

    /// Set current tool to pending (requires user confirmation).
    pub fn pending(&mut self, interaction: Interaction) {
        if let Some(ref mut tool) = self.tool {
            tool.pending = true;
            tool.pending_interaction = Some(interaction);
        }
    }

    /// Confirm pending tool execution.
    pub fn confirm(&mut self) {
        if let Some(ref mut tool) = self.tool {
            tool.pending = false;
            tool.pending_interaction = None;
        }
    }

    /// Set tool result.
    pub fn set_tool_result(&mut self, result: ToolResult) {
        if let Some(ref mut tool) = self.tool {
            tool.result = Some(result);
        }
    }

    // =========================================================================
    // Step Outcome
    // =========================================================================

    /// Get the step outcome based on current state.
    pub fn result(&self) -> StepOutcome {
        // Check if any tool is pending
        if let Some(ref tool) = self.tool {
            if tool.pending {
                if let Some(ref interaction) = tool.pending_interaction {
                    return StepOutcome::Pending(interaction.clone());
                }
            }
        }

        // Check if LLM response has more tool calls or is complete
        if let Some(ref response) = self.response {
            if response.tool_calls.is_empty() && !response.text.is_empty() {
                return StepOutcome::Complete;
            }
        }

        StepOutcome::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestFixture;
    use serde_json::json;

    fn mock_tools() -> Vec<ToolDescriptor> {
        vec![
            ToolDescriptor::new("read_file", "Read File", "Read a file"),
            ToolDescriptor::new("write_file", "Write File", "Write a file"),
            ToolDescriptor::new("delete_file", "Delete File", "Delete a file"),
        ]
    }

    // =========================================================================
    // Phase tests
    // =========================================================================

    #[test]
    fn test_phase_display() {
        assert_eq!(Phase::RunStart.to_string(), "RunStart");
        assert_eq!(Phase::StepStart.to_string(), "StepStart");
        assert_eq!(Phase::BeforeInference.to_string(), "BeforeInference");
        assert_eq!(Phase::AfterInference.to_string(), "AfterInference");
        assert_eq!(Phase::BeforeToolExecute.to_string(), "BeforeToolExecute");
        assert_eq!(Phase::AfterToolExecute.to_string(), "AfterToolExecute");
        assert_eq!(Phase::StepEnd.to_string(), "StepEnd");
        assert_eq!(Phase::RunEnd.to_string(), "RunEnd");
    }

    #[test]
    fn test_phase_equality() {
        assert_eq!(Phase::RunStart, Phase::RunStart);
        assert_ne!(Phase::RunStart, Phase::RunEnd);
    }

    #[test]
    fn test_phase_clone() {
        let phase = Phase::BeforeInference;
        let cloned = phase;
        assert_eq!(phase, cloned);
    }

    #[test]
    fn test_phase_policy() {
        let before_inference = Phase::BeforeInference.policy();
        assert!(before_inference.allow_tool_filter_mutation);
        assert!(before_inference.allow_skip_inference_mutation);
        assert!(!before_inference.allow_tool_gate_mutation);

        let before_tool_execute = Phase::BeforeToolExecute.policy();
        assert!(!before_tool_execute.allow_tool_filter_mutation);
        assert!(!before_tool_execute.allow_skip_inference_mutation);
        assert!(before_tool_execute.allow_tool_gate_mutation);

        let run_end = Phase::RunEnd.policy();
        assert_eq!(run_end, PhasePolicy::read_only());
    }

    // =========================================================================
    // StepContext tests
    // =========================================================================

    #[test]
    fn test_step_context_new() {
        let fix = TestFixture::new();
        let ctx = fix.step(mock_tools());

        assert!(ctx.system_context.is_empty());
        assert!(ctx.session_context.is_empty());
        assert!(ctx.system_reminders.is_empty());
        assert_eq!(ctx.tools.len(), 3);
        assert!(ctx.tool.is_none());
        assert!(ctx.response.is_none());
        assert!(!ctx.skip_inference);
    }

    #[test]
    fn test_step_context_reset() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(mock_tools());

        ctx.system("test");
        ctx.thread("test");
        ctx.reminder("test");
        ctx.skip_inference = true;

        ctx.reset();

        assert!(ctx.system_context.is_empty());
        assert!(ctx.session_context.is_empty());
        assert!(ctx.system_reminders.is_empty());
        assert!(!ctx.skip_inference);
    }

    // =========================================================================
    // Context injection tests
    // =========================================================================

    #[test]
    fn test_system_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.system("Context 1");
        ctx.system("Context 2");

        assert_eq!(ctx.system_context.len(), 2);
        assert_eq!(ctx.system_context[0], "Context 1");
        assert_eq!(ctx.system_context[1], "Context 2");
    }

    #[test]
    fn test_set_system_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.system("Context 1");
        ctx.system("Context 2");
        ctx.set_system("Replaced");

        assert_eq!(ctx.system_context.len(), 1);
        assert_eq!(ctx.system_context[0], "Replaced");
    }

    #[test]
    fn test_clear_system_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.system("Context 1");
        ctx.clear_system();

        assert!(ctx.system_context.is_empty());
    }

    #[test]
    fn test_session_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.thread("AgentState 1");
        ctx.thread("AgentState 2");

        assert_eq!(ctx.session_context.len(), 2);
    }

    #[test]
    fn test_set_session_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.thread("AgentState 1");
        ctx.set_session("Replaced");

        assert_eq!(ctx.session_context.len(), 1);
        assert_eq!(ctx.session_context[0], "Replaced");
    }

    #[test]
    fn test_reminder() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.reminder("Reminder 1");
        ctx.reminder("Reminder 2");

        assert_eq!(ctx.system_reminders.len(), 2);
    }

    #[test]
    fn test_clear_reminders() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.reminder("Reminder 1");
        ctx.clear_reminders();

        assert!(ctx.system_reminders.is_empty());
    }

    // =========================================================================
    // Tool filtering tests
    // =========================================================================

    #[test]
    fn test_exclude_tool() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(mock_tools());

        ctx.exclude("delete_file");

        assert_eq!(ctx.tools.len(), 2);
        assert!(ctx.tools.iter().all(|t| t.id != "delete_file"));
    }

    #[test]
    fn test_include_only_tools() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(mock_tools());

        ctx.include_only(&["read_file"]);

        assert_eq!(ctx.tools.len(), 1);
        assert_eq!(ctx.tools[0].id, "read_file");
    }

    // =========================================================================
    // Tool control tests
    // =========================================================================

    #[test]
    fn test_tool_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({"path": "/test"}));
        ctx.tool = Some(ToolContext::new(&call));

        assert_eq!(ctx.tool_name(), Some("read_file"));
        assert_eq!(ctx.tool_call_id(), Some("call_1"));
        assert_eq!(ctx.tool_idempotency_key(), Some("call_1"));
        assert_eq!(ctx.tool_args().unwrap()["path"], "/test");
        assert!(!ctx.tool_blocked());
        assert!(!ctx.tool_pending());
    }

    #[test]
    fn test_block_tool() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "delete_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        ctx.block("Permission denied");

        assert!(ctx.tool_blocked());
        assert_eq!(
            ctx.tool.as_ref().unwrap().block_reason,
            Some("Permission denied".to_string())
        );
    }

    #[test]
    fn test_pending_tool() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        let interaction = Interaction::new("confirm_1", "confirm").with_message("Allow write?");
        ctx.pending(interaction);

        assert!(ctx.tool_pending());
        assert!(ctx.tool.as_ref().unwrap().pending_interaction.is_some());
    }

    #[test]
    fn test_confirm_tool() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        let interaction = Interaction::new("confirm_1", "confirm").with_message("Allow write?");
        ctx.pending(interaction);
        ctx.confirm();

        assert!(!ctx.tool_pending());
        assert!(ctx.tool.as_ref().unwrap().pending_interaction.is_none());
    }

    #[test]
    fn test_set_tool_result() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "read_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        let result = ToolResult::success("read_file", json!({"content": "hello"}));
        ctx.set_tool_result(result);

        assert!(ctx.tool_result().is_some());
        assert!(ctx.tool_result().unwrap().is_success());
    }

    // =========================================================================
    // StepOutcome tests
    // =========================================================================

    #[test]
    fn test_step_result_continue() {
        let fix = TestFixture::new();
        let ctx = fix.step(vec![]);

        assert_eq!(ctx.result(), StepOutcome::Continue);
    }

    #[test]
    fn test_step_result_pending() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        let interaction = Interaction::new("confirm_1", "confirm").with_message("Allow?");
        ctx.pending(interaction.clone());

        match ctx.result() {
            StepOutcome::Pending(i) => assert_eq!(i.id, "confirm_1"),
            _ => panic!("Expected Pending result"),
        }
    }

    #[test]
    fn test_step_result_complete() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.response = Some(StreamResult {
            text: "Done!".to_string(),
            tool_calls: vec![],
            usage: None,
        });

        assert_eq!(ctx.result(), StepOutcome::Complete);
    }

    // =========================================================================
    // ToolContext tests
    // =========================================================================

    #[test]
    fn test_tool_context_new() {
        let call = ToolCall::new("call_1", "test_tool", json!({"arg": "value"}));
        let tool_ctx = ToolContext::new(&call);

        assert_eq!(tool_ctx.id, "call_1");
        assert_eq!(tool_ctx.idempotency_key(), "call_1");
        assert_eq!(tool_ctx.name, "test_tool");
        assert_eq!(tool_ctx.args["arg"], "value");
        assert!(tool_ctx.result.is_none());
        assert!(!tool_ctx.blocked);
        assert!(!tool_ctx.pending);
    }

    #[test]
    fn test_tool_context_is_blocked() {
        let call = ToolCall::new("call_1", "test", json!({}));
        let mut tool_ctx = ToolContext::new(&call);

        assert!(!tool_ctx.is_blocked());
        tool_ctx.blocked = true;
        assert!(tool_ctx.is_blocked());
    }

    #[test]
    fn test_tool_context_is_pending() {
        let call = ToolCall::new("call_1", "test", json!({}));
        let mut tool_ctx = ToolContext::new(&call);

        assert!(!tool_ctx.is_pending());
        tool_ctx.pending = true;
        assert!(tool_ctx.is_pending());
    }

    // =========================================================================
    // Additional edge case tests
    // =========================================================================

    #[test]
    fn test_phase_all_8_values() {
        let phases = [
            Phase::RunStart,
            Phase::StepStart,
            Phase::BeforeInference,
            Phase::AfterInference,
            Phase::BeforeToolExecute,
            Phase::AfterToolExecute,
            Phase::StepEnd,
            Phase::RunEnd,
        ];

        assert_eq!(phases.len(), 8);
        // All should be unique
        for (i, p1) in phases.iter().enumerate() {
            for (j, p2) in phases.iter().enumerate() {
                if i != j {
                    assert_ne!(p1, p2);
                }
            }
        }
    }

    #[test]
    fn test_step_context_empty_session() {
        let fix = TestFixture::new();
        let ctx = fix.step(vec![]);

        assert!(ctx.tools.is_empty());
        assert!(ctx.system_context.is_empty());
        assert_eq!(ctx.result(), StepOutcome::Continue);
    }

    #[test]
    fn test_step_context_multiple_system_contexts() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.system("Context 1");
        ctx.system("Context 2");
        ctx.system("Context 3");

        assert_eq!(ctx.system_context.len(), 3);
        assert_eq!(ctx.system_context[0], "Context 1");
        assert_eq!(ctx.system_context[2], "Context 3");
    }

    #[test]
    fn test_step_context_multiple_session_contexts() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.thread("AgentState 1");
        ctx.thread("AgentState 2");

        assert_eq!(ctx.session_context.len(), 2);
    }

    #[test]
    fn test_step_context_multiple_reminders() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.reminder("Reminder 1");
        ctx.reminder("Reminder 2");
        ctx.reminder("Reminder 3");

        assert_eq!(ctx.system_reminders.len(), 3);
    }

    #[test]
    fn test_exclude_nonexistent_tool() {
        let fix = TestFixture::new();
        let tools = mock_tools();
        let original_len = tools.len();
        let mut ctx = fix.step(tools);

        ctx.exclude("nonexistent_tool");

        // Should not change anything
        assert_eq!(ctx.tools.len(), original_len);
    }

    #[test]
    fn test_exclude_multiple_tools() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(mock_tools());

        ctx.exclude("read_file");
        ctx.exclude("delete_file");

        assert_eq!(ctx.tools.len(), 1);
        assert_eq!(ctx.tools[0].id, "write_file");
    }

    #[test]
    fn test_include_only_empty_list() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(mock_tools());

        ctx.include_only(&[]);

        assert!(ctx.tools.is_empty());
    }

    #[test]
    fn test_include_only_with_nonexistent() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(mock_tools());

        ctx.include_only(&["read_file", "nonexistent"]);

        assert_eq!(ctx.tools.len(), 1);
        assert_eq!(ctx.tools[0].id, "read_file");
    }

    #[test]
    fn test_block_without_tool_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        // No tool context, block should not panic
        ctx.block("test");

        assert!(!ctx.tool_blocked()); // tool_blocked returns false when no tool
    }

    #[test]
    fn test_pending_without_tool_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let interaction = Interaction::new("id", "confirm").with_message("test");
        ctx.pending(interaction);

        assert!(!ctx.tool_pending()); // tool_pending returns false when no tool
    }

    #[test]
    fn test_confirm_without_pending() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "test", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        // Confirm without pending should not panic
        ctx.confirm();

        assert!(!ctx.tool_pending());
    }

    #[test]
    fn test_tool_args_without_tool() {
        let fix = TestFixture::new();
        let ctx = fix.step(vec![]);

        assert!(ctx.tool_args().is_none());
    }

    #[test]
    fn test_tool_name_without_tool() {
        let fix = TestFixture::new();
        let ctx = fix.step(vec![]);

        assert!(ctx.tool_name().is_none());
        assert!(ctx.tool_call_id().is_none());
        assert!(ctx.tool_idempotency_key().is_none());
    }

    #[test]
    fn test_tool_result_without_tool() {
        let fix = TestFixture::new();
        let ctx = fix.step(vec![]);

        assert!(ctx.tool_result().is_none());
    }

    #[test]
    fn test_step_result_with_tool_calls() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.response = Some(StreamResult {
            text: "Calling tools".to_string(),
            tool_calls: vec![ToolCall::new("call_1", "test", json!({}))],
            usage: None,
        });

        // With tool calls, should continue
        assert_eq!(ctx.result(), StepOutcome::Continue);
    }

    #[test]
    fn test_step_result_empty_text_no_tools() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.response = Some(StreamResult {
            text: String::new(),
            tool_calls: vec![],
            usage: None,
        });

        // Empty text, no tool calls -> Continue (not Complete)
        assert_eq!(ctx.result(), StepOutcome::Continue);
    }

    #[test]
    fn test_tool_context_block_reason() {
        let call = ToolCall::new("call_1", "test", json!({}));
        let mut tool_ctx = ToolContext::new(&call);

        assert!(tool_ctx.block_reason.is_none());
        tool_ctx.block_reason = Some("Test reason".to_string());
        assert_eq!(tool_ctx.block_reason, Some("Test reason".to_string()));
    }

    #[test]
    fn test_tool_context_pending_interaction() {
        let call = ToolCall::new("call_1", "test", json!({}));
        let mut tool_ctx = ToolContext::new(&call);

        assert!(tool_ctx.pending_interaction.is_none());

        let interaction = Interaction::new("confirm_1", "confirm").with_message("Test?");
        tool_ctx.pending_interaction = Some(interaction.clone());

        assert_eq!(
            tool_ctx.pending_interaction.as_ref().unwrap().id,
            "confirm_1"
        );
    }

    #[test]
    fn test_set_clear_session_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.thread("Context 1");
        ctx.thread("Context 2");
        ctx.set_session("Only this");

        assert_eq!(ctx.session_context.len(), 1);
        assert_eq!(ctx.session_context[0], "Only this");
    }
}
