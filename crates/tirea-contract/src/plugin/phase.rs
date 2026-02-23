//! Phase-based plugin execution system.
//!
//! This module provides the core types for the plugin phase system:
//! - `Phase`: Execution phases in the agent loop
//! - `StepContext`: Mutable context passed through all phases
//! - `ToolContext`: Tool-call state carried by `StepContext`

use crate::event::suspension::{FrontendToolInvocation, Suspension};
#[cfg(test)]
use crate::event::suspension::{InvocationOrigin, ResponseRouting};
use crate::event::termination::TerminationReason;
use crate::runtime::result::StreamResult;
use crate::thread::{Message, ToolCall};
use crate::tool::context::ToolCallContext;
use crate::tool::contract::{ToolDescriptor, ToolResult};
use crate::RunConfig;
use serde_json::Value;
use std::sync::Arc;
use tirea_state::{State, TireaResult, TrackedPatch};

/// Execution phase in the agent loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Thread started (called once).
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
    /// Thread ended (called once).
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
    /// Whether `StepContext::termination_request` can be mutated.
    pub allow_termination_request_mutation: bool,
    /// Whether tool execution gate (`blocked/pending`) can be mutated.
    pub allow_tool_gate_mutation: bool,
}

impl PhasePolicy {
    pub const fn read_only() -> Self {
        Self {
            allow_tool_filter_mutation: false,
            allow_skip_inference_mutation: false,
            allow_termination_request_mutation: false,
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
                allow_termination_request_mutation: true,
                allow_tool_gate_mutation: false,
            },
            Self::AfterInference => PhasePolicy {
                allow_tool_filter_mutation: false,
                allow_skip_inference_mutation: false,
                allow_termination_request_mutation: true,
                allow_tool_gate_mutation: false,
            },
            Self::BeforeToolExecute => PhasePolicy {
                allow_tool_filter_mutation: false,
                allow_skip_inference_mutation: false,
                allow_termination_request_mutation: false,
                allow_tool_gate_mutation: true,
            },
            Self::RunStart
            | Self::StepStart
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
    /// Thread complete.
    Complete,
    /// Pending external suspension.
    Pending(Suspension),
}

/// Run-level control action emitted by plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAction {
    /// Continue normal execution.
    Continue,
    /// Terminate run with specific reason.
    Terminate(TerminationReason),
}

/// Suspension payload for `ToolGateDecision::Suspend`.
#[derive(Debug, Clone, PartialEq)]
pub struct SuspendTicket {
    pub suspension: Suspension,
    pub invocation: Option<FrontendToolInvocation>,
}

impl SuspendTicket {
    pub fn new(suspension: Suspension) -> Self {
        Self {
            suspension,
            invocation: None,
        }
    }

    pub fn from_invocation(invocation: FrontendToolInvocation) -> Self {
        let suspension = invocation.to_suspension();
        Self {
            suspension,
            invocation: Some(invocation),
        }
    }

    pub fn with_invocation(mut self, invocation: FrontendToolInvocation) -> Self {
        self.invocation = Some(invocation);
        self
    }
}

/// Tool gate decision for `BeforeToolExecute`.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolGateDecision {
    Proceed,
    Suspend(SuspendTicket),
    Cancel { reason: String },
}

/// Tool-level control action emitted by plugins.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolAction {
    Proceed,
    Suspend(SuspendTicket),
    Cancel { reason: String },
}

/// State side effect emitted by plugins.
#[derive(Debug, Clone)]
pub enum StateEffect {
    Patch(TrackedPatch),
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
    /// Canonical suspend ticket carrying pause payload.
    pub suspend_ticket: Option<SuspendTicket>,
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
            suspend_ticket: None,
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
    /// Execution context providing state access, run config, identity.
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
    /// Legacy skip-inference field.
    ///
    /// Deprecated runtime behavior: this field is no longer consumed by loop control.
    /// Plugins must use `run_action` via typed phase APIs.
    pub skip_inference: bool,
    /// Legacy termination request field.
    ///
    /// Deprecated runtime behavior: this field is no longer consumed by loop control.
    /// Plugins must use `run_action` via typed phase APIs.
    pub termination_request: Option<TerminationReason>,
    /// Unified run-level action emitted by plugins.
    pub run_action: Option<RunAction>,

    // === Pending State Changes ===
    /// Patches to apply to session state after this phase completes.
    pub pending_patches: Vec<TrackedPatch>,
    /// State effects emitted by plugins.
    pub state_effects: Vec<StateEffect>,
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
            termination_request: None,
            run_action: None,
            pending_patches: Vec::new(),
            state_effects: Vec::new(),
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

    /// Borrow the run config.
    pub fn run_config(&self) -> &RunConfig {
        self.ctx.run_config()
    }

    /// Typed run config accessor.
    pub fn config_state<T: State>(&self) -> TireaResult<T::Ref<'_>> {
        self.ctx.config_state::<T>()
    }

    /// Snapshot the current document state.
    pub fn snapshot(&self) -> Value {
        self.ctx.snapshot()
    }

    /// Typed snapshot at the type's canonical path.
    pub fn snapshot_of<T: State>(&self) -> TireaResult<T> {
        self.ctx.snapshot_of::<T>()
    }

    /// Typed snapshot at an explicit path.
    pub fn snapshot_at<T: State>(&self, path: &str) -> TireaResult<T> {
        self.ctx.snapshot_at::<T>(path)
    }

    /// Reset step-specific state for a new step.
    pub fn reset(&mut self) {
        self.system_context.clear();
        self.session_context.clear();
        self.system_reminders.clear();
        self.tool = None;
        self.response = None;
        self.skip_inference = false;
        self.termination_request = None;
        self.run_action = None;
        self.pending_patches.clear();
        self.state_effects.clear();
    }

    // =========================================================================
    // Context Injection [Position 1, 2, 7]
    // =========================================================================

    /// Add system context (appended to system prompt) [Position 1].
    pub fn system(&mut self, content: impl Into<String>) {
        self.system_context.push(content.into());
    }

    /// Add session context message (before user messages) [Position 2].
    pub fn thread(&mut self, content: impl Into<String>) {
        self.session_context.push(content.into());
    }

    /// Add system reminder (after tool result) [Position 7].
    pub fn reminder(&mut self, content: impl Into<String>) {
        self.system_reminders.push(content.into());
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

    /// Mark the current tool as explicitly allowed.
    ///
    /// This clears any prior cancel/suspend state.
    pub fn proceed(&mut self) {
        if let Some(ref mut tool) = self.tool {
            tool.blocked = false;
            tool.block_reason = None;
            tool.pending = false;
            tool.suspend_ticket = None;
        }
    }

    /// Mark the current tool as canceled with a reason.
    ///
    /// This clears any prior suspend state.
    pub fn cancel(&mut self, reason: impl Into<String>) {
        if let Some(ref mut tool) = self.tool {
            tool.blocked = true;
            tool.block_reason = Some(reason.into());
            tool.pending = false;
            tool.suspend_ticket = None;
        }
    }

    /// Mark the current tool as suspended with a ticket.
    ///
    /// This clears any prior cancel state.
    pub fn suspend(&mut self, ticket: SuspendTicket) {
        if let Some(ref mut tool) = self.tool {
            tool.blocked = false;
            tool.block_reason = None;
            tool.pending = true;
            tool.suspend_ticket = Some(ticket);
        }
    }

    /// Set tool result.
    pub fn set_tool_result(&mut self, result: ToolResult) {
        if let Some(ref mut tool) = self.tool {
            tool.result = Some(result);
        }
    }

    /// Set run-level action.
    pub fn set_run_action(&mut self, action: RunAction) {
        self.run_action = Some(action);
    }

    /// Emit a state side effect.
    pub fn emit_state_effect(&mut self, effect: StateEffect) {
        self.state_effects.push(effect);
    }

    /// Emit a state patch side effect.
    pub fn emit_patch(&mut self, patch: TrackedPatch) {
        self.emit_state_effect(StateEffect::Patch(patch));
    }

    /// Effective run-level action for current step.
    pub fn run_action(&self) -> RunAction {
        if let Some(action) = self.run_action.clone() {
            return action;
        }
        RunAction::Continue
    }

    /// Current tool action derived from tool gate state.
    pub fn tool_action(&self) -> ToolAction {
        if let Some(tool) = self.tool.as_ref() {
            if tool.blocked {
                return ToolAction::Cancel {
                    reason: tool.block_reason.clone().unwrap_or_default(),
                };
            }
            if tool.pending {
                if let Some(ticket) = tool.suspend_ticket.as_ref() {
                    return ToolAction::Suspend(ticket.clone());
                }
                return ToolAction::Suspend(SuspendTicket::new(Suspension::new(
                    tool.id.clone(),
                    format!("tool:{}", tool.name),
                )));
            }
        }
        ToolAction::Proceed
    }

    // =========================================================================
    // Step Outcome
    // =========================================================================

    /// Get the step outcome based on current state.
    pub fn result(&self) -> StepOutcome {
        // Check if any tool is pending
        if let Some(ref tool) = self.tool {
            if tool.pending {
                if let Some(ticket) = tool.suspend_ticket.as_ref() {
                    return StepOutcome::Pending(ticket.suspension.clone());
                }
                return StepOutcome::Pending(Suspension::new(
                    tool.id.clone(),
                    format!("tool:{}", tool.name),
                ));
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

/// Shared read access available to all phase contexts.
pub trait PluginPhaseContext {
    fn phase(&self) -> Phase;
    fn thread_id(&self) -> &str;
    fn messages(&self) -> &[Arc<Message>];
    fn run_config(&self) -> &RunConfig;
    fn config_value(&self, key: &str) -> Option<&Value> {
        self.run_config().value(key)
    }
    fn state_of<T: State>(&self) -> T::Ref<'_>;
    fn snapshot(&self) -> Value;
}

macro_rules! impl_plugin_phase_context {
    ($name:ident, $phase:expr) => {
        impl<'s, 'a> $name<'s, 'a> {
            pub fn new(step: &'s mut StepContext<'a>) -> Self {
                Self { step }
            }

            #[cfg(feature = "test-support")]
            pub fn step_mut_for_tests(&mut self) -> &mut StepContext<'a> {
                self.step
            }
        }

        impl<'s, 'a> PluginPhaseContext for $name<'s, 'a> {
            fn phase(&self) -> Phase {
                $phase
            }

            fn thread_id(&self) -> &str {
                self.step.thread_id()
            }

            fn messages(&self) -> &[Arc<Message>] {
                self.step.messages()
            }

            fn run_config(&self) -> &RunConfig {
                self.step.run_config()
            }

            fn state_of<T: State>(&self) -> T::Ref<'_> {
                self.step.state_of::<T>()
            }

            fn snapshot(&self) -> Value {
                self.step.snapshot()
            }
        }
    };
}

pub struct RunStartContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(RunStartContext, Phase::RunStart);

pub struct StepStartContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(StepStartContext, Phase::StepStart);

pub struct BeforeInferenceContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(BeforeInferenceContext, Phase::BeforeInference);

impl<'s, 'a> BeforeInferenceContext<'s, 'a> {
    /// Append a system context line.
    pub fn add_system_context(&mut self, text: impl Into<String>) {
        self.step.system(text);
    }

    /// Append a session message.
    pub fn add_session_message(&mut self, text: impl Into<String>) {
        self.step.thread(text);
    }

    /// Exclude tool by id.
    pub fn exclude_tool(&mut self, tool_id: &str) {
        self.step.exclude(tool_id);
    }

    /// Keep only listed tools.
    pub fn include_only(&mut self, tool_ids: &[&str]) {
        self.step.include_only(tool_ids);
    }

    /// Skip current inference.
    pub fn skip_inference(&mut self) {
        self.step
            .set_run_action(RunAction::Terminate(TerminationReason::PluginRequested));
    }

    /// Request run termination with a specific reason.
    pub fn request_termination(&mut self, reason: TerminationReason) {
        self.step.set_run_action(RunAction::Terminate(reason));
    }
}

pub struct AfterInferenceContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(AfterInferenceContext, Phase::AfterInference);

impl<'s, 'a> AfterInferenceContext<'s, 'a> {
    pub fn response_opt(&self) -> Option<&StreamResult> {
        self.step.response.as_ref()
    }

    pub fn response(&self) -> &StreamResult {
        self.step
            .response
            .as_ref()
            .expect("AfterInferenceContext.response() requires response to be set")
    }

    /// Request run termination with a specific reason after inference has completed.
    pub fn request_termination(&mut self, reason: TerminationReason) {
        self.step.set_run_action(RunAction::Terminate(reason));
    }
}

pub struct BeforeToolExecuteContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(BeforeToolExecuteContext, Phase::BeforeToolExecute);

impl<'s, 'a> BeforeToolExecuteContext<'s, 'a> {
    pub fn tool_name(&self) -> Option<&str> {
        self.step.tool_name()
    }

    pub fn tool_call_id(&self) -> Option<&str> {
        self.step.tool_call_id()
    }

    pub fn tool_args(&self) -> Option<&Value> {
        self.step.tool_args()
    }

    pub fn decision(&self) -> ToolGateDecision {
        match self.step.tool_action() {
            ToolAction::Proceed => ToolGateDecision::Proceed,
            ToolAction::Suspend(ticket) => ToolGateDecision::Suspend(ticket),
            ToolAction::Cancel { reason } => ToolGateDecision::Cancel { reason },
        }
    }

    pub fn cancel(&mut self, reason: impl Into<String>) {
        self.step.cancel(reason);
    }

    /// Explicitly proceed with tool execution.
    ///
    /// This clears any previous cancel/suspend state set by earlier plugins.
    pub fn proceed(&mut self) {
        self.step.proceed();
    }

    pub fn suspend(&mut self, ticket: SuspendTicket) {
        self.step.suspend(ticket);
    }
}

pub struct AfterToolExecuteContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(AfterToolExecuteContext, Phase::AfterToolExecute);

impl<'s, 'a> AfterToolExecuteContext<'s, 'a> {
    pub fn tool_name(&self) -> Option<&str> {
        self.step.tool_name()
    }

    pub fn tool_call_id(&self) -> Option<&str> {
        self.step.tool_call_id()
    }

    pub fn tool_result(&self) -> &ToolResult {
        self.step
            .tool_result()
            .expect("AfterToolExecuteContext.tool_result() requires tool result")
    }

    pub fn add_system_reminder(&mut self, text: impl Into<String>) {
        self.step.reminder(text);
    }
}

pub struct StepEndContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(StepEndContext, Phase::StepEnd);

pub struct RunEndContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_plugin_phase_context!(RunEndContext, Phase::RunEnd);

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
        assert!(before_inference.allow_termination_request_mutation);
        assert!(!before_inference.allow_tool_gate_mutation);

        let after_inference = Phase::AfterInference.policy();
        assert!(!after_inference.allow_tool_filter_mutation);
        assert!(!after_inference.allow_skip_inference_mutation);
        assert!(after_inference.allow_termination_request_mutation);
        assert!(!after_inference.allow_tool_gate_mutation);

        let before_tool_execute = Phase::BeforeToolExecute.policy();
        assert!(!before_tool_execute.allow_tool_filter_mutation);
        assert!(!before_tool_execute.allow_skip_inference_mutation);
        assert!(!before_tool_execute.allow_termination_request_mutation);
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
        assert!(ctx.termination_request.is_none());
        assert!(ctx.run_action.is_none());
        assert!(ctx.state_effects.is_empty());
    }

    #[test]
    fn test_step_context_reset() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(mock_tools());

        ctx.system("test");
        ctx.thread("test");
        ctx.reminder("test");
        ctx.skip_inference = true;
        ctx.termination_request = Some(TerminationReason::PluginRequested);

        ctx.reset();

        assert!(ctx.system_context.is_empty());
        assert!(ctx.session_context.is_empty());
        assert!(ctx.system_reminders.is_empty());
        assert!(!ctx.skip_inference);
        assert!(ctx.termination_request.is_none());
        assert!(ctx.run_action.is_none());
        assert!(ctx.state_effects.is_empty());
    }

    #[test]
    fn test_after_inference_request_termination_does_not_set_skip_inference() {
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);
        {
            let mut ctx = AfterInferenceContext::new(&mut step);
            ctx.request_termination(TerminationReason::PluginRequested);
        }
        assert!(step.termination_request.is_none());
        assert!(!step.skip_inference);
        assert_eq!(
            step.run_action,
            Some(RunAction::Terminate(TerminationReason::PluginRequested))
        );
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
        ctx.system_context = vec!["Replaced".to_string()];

        assert_eq!(ctx.system_context.len(), 1);
        assert_eq!(ctx.system_context[0], "Replaced");
    }

    #[test]
    fn test_clear_system_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.system("Context 1");
        ctx.system_context.clear();

        assert!(ctx.system_context.is_empty());
    }

    #[test]
    fn test_session_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.thread("Thread 1");
        ctx.thread("Thread 2");

        assert_eq!(ctx.session_context.len(), 2);
    }

    #[test]
    fn test_set_session_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.thread("Thread 1");
        ctx.session_context = vec!["Replaced".to_string()];

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
        ctx.system_reminders.clear();

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

        ctx.cancel("Permission denied");

        assert!(ctx.tool_blocked());
        assert!(!ctx.tool_pending());
        assert_eq!(
            ctx.tool.as_ref().unwrap().block_reason,
            Some("Permission denied".to_string())
        );
        assert!(ctx.tool.as_ref().unwrap().suspend_ticket.is_none());
    }

    #[test]
    fn test_pending_tool() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        let interaction = Suspension::new("confirm_1", "confirm").with_message("Allow write?");
        ctx.suspend(SuspendTicket::new(interaction));

        assert!(ctx.tool_pending());
        assert!(!ctx.tool_blocked());
        assert!(ctx.tool.as_ref().unwrap().block_reason.is_none());
        assert!(ctx.tool.as_ref().unwrap().suspend_ticket.is_some());
    }

    #[test]
    fn test_confirm_tool() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        let interaction = Suspension::new("confirm_1", "confirm").with_message("Allow write?");
        ctx.suspend(SuspendTicket::new(interaction));
        ctx.proceed();

        assert!(!ctx.tool_pending());
        assert!(!ctx.tool_blocked());
        assert!(ctx.tool.as_ref().unwrap().block_reason.is_none());
        assert!(ctx.tool.as_ref().unwrap().suspend_ticket.is_none());
    }

    #[test]
    fn test_allow_deny_ask_transitions_are_mutually_exclusive() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        ctx.cancel("denied");
        assert!(ctx.tool_blocked());
        assert!(!ctx.tool_pending());

        ctx.suspend(SuspendTicket::new(
            Suspension::new("confirm_1", "confirm").with_message("Allow write?"),
        ));
        assert!(!ctx.tool_blocked());
        assert!(ctx.tool_pending());
        assert!(ctx.tool.as_ref().unwrap().block_reason.is_none());

        ctx.proceed();
        assert!(!ctx.tool_blocked());
        assert!(!ctx.tool_pending());
        assert!(ctx.tool.as_ref().unwrap().suspend_ticket.is_none());
        assert!(ctx
            .tool
            .as_ref()
            .unwrap()
            .suspend_ticket
            .as_ref()
            .and_then(|ticket| ticket.invocation.as_ref())
            .is_none());
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

        let interaction = Suspension::new("confirm_1", "confirm").with_message("Allow?");
        ctx.suspend(SuspendTicket::new(interaction.clone()));

        match ctx.result() {
            StepOutcome::Pending(i) => assert_eq!(i.id, "confirm_1"),
            _ => panic!("Expected Pending result"),
        }
    }

    #[test]
    fn test_step_result_pending_prefers_suspend_ticket() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        let ticket_interaction =
            Suspension::new("ticket_1", "confirm").with_message("Suspend via ticket");

        if let Some(tool) = ctx.tool.as_mut() {
            tool.pending = true;
            tool.suspend_ticket = Some(SuspendTicket::new(ticket_interaction.clone()));
        }

        match ctx.result() {
            StepOutcome::Pending(interaction) => {
                assert_eq!(interaction.id, ticket_interaction.id);
            }
            other => panic!("Expected Pending result, got: {other:?}"),
        }
    }

    #[test]
    fn test_before_tool_execute_decision_prefers_suspend_ticket() {
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);

        let call = ToolCall::new("call_1", "write_file", json!({}));
        step.tool = Some(ToolContext::new(&call));

        let ticket_interaction =
            Suspension::new("ticket_2", "confirm").with_message("Suspend via ticket");

        if let Some(tool) = step.tool.as_mut() {
            tool.pending = true;
            tool.suspend_ticket = Some(SuspendTicket::new(ticket_interaction.clone()));
        }

        let ctx = BeforeToolExecuteContext::new(&mut step);
        match ctx.decision() {
            ToolGateDecision::Suspend(ticket) => {
                assert_eq!(ticket.suspension.id, ticket_interaction.id);
            }
            other => panic!("Expected Suspend decision, got: {other:?}"),
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

        ctx.thread("Thread 1");
        ctx.thread("Thread 2");

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
        ctx.cancel("test");

        assert!(!ctx.tool_blocked()); // tool_blocked returns false when no tool
    }

    #[test]
    fn test_pending_without_tool_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let interaction = Suspension::new("id", "confirm").with_message("test");
        ctx.suspend(SuspendTicket::new(interaction));

        assert!(!ctx.tool_pending()); // tool_pending returns false when no tool
    }

    #[test]
    fn test_confirm_without_pending() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_1", "test", json!({}));
        ctx.tool = Some(ToolContext::new(&call));

        // Confirm without pending should not panic
        ctx.proceed();

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
    fn test_tool_context_suspend_ticket() {
        let call = ToolCall::new("call_1", "test", json!({}));
        let mut tool_ctx = ToolContext::new(&call);

        assert!(tool_ctx.suspend_ticket.is_none());

        let interaction = Suspension::new("confirm_1", "confirm").with_message("Test?");
        tool_ctx.suspend_ticket = Some(SuspendTicket::new(interaction.clone()));

        assert_eq!(
            tool_ctx.suspend_ticket.as_ref().unwrap().suspension.id,
            "confirm_1"
        );
    }

    #[test]
    fn test_suspend_with_invocation_direct() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_copy", "copyToClipboard", json!({"text": "hello"}));
        ctx.tool = Some(ToolContext::new(&call));
        ctx.cancel("old deny state");

        let invocation = FrontendToolInvocation::new(
            "call_copy",
            "copyToClipboard",
            json!({"text": "hello"}),
            InvocationOrigin::PluginInitiated {
                plugin_id: "agui_frontend_tools".to_string(),
            },
            ResponseRouting::UseAsToolResult,
        );
        ctx.suspend(SuspendTicket::from_invocation(invocation));

        assert!(ctx.tool_pending());
        assert!(!ctx.tool_blocked());
        assert!(ctx.tool.as_ref().unwrap().block_reason.is_none());

        let invocation = ctx
            .tool
            .as_ref()
            .unwrap()
            .suspend_ticket
            .as_ref()
            .and_then(|ticket| ticket.invocation.as_ref())
            .expect("pending frontend invocation should exist");
        assert_eq!(invocation.call_id, "call_copy");
        assert_eq!(invocation.tool_name, "copyToClipboard");
        assert!(matches!(
            invocation.routing,
            ResponseRouting::UseAsToolResult
        ));
        assert!(matches!(
            invocation.origin,
            InvocationOrigin::PluginInitiated { .. }
        ));

        // Backward-compat Suspension should also be set
        let interaction = ctx
            .tool
            .as_ref()
            .unwrap()
            .suspend_ticket
            .as_ref()
            .map(|ticket| &ticket.suspension)
            .unwrap();
        assert_eq!(interaction.id, "call_copy");
        assert_eq!(interaction.action, "tool:copyToClipboard");
    }

    #[test]
    fn test_suspend_with_invocation_replay_original_tool() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let call = ToolCall::new("call_write", "write_file", json!({"path": "a.txt"}));
        ctx.tool = Some(ToolContext::new(&call));

        let call_id = "fc_generated";
        let invocation = FrontendToolInvocation::new(
            call_id,
            "PermissionConfirm",
            json!({"tool_name": "write_file", "tool_args": {"path": "a.txt"}}),
            InvocationOrigin::ToolCallIntercepted {
                backend_call_id: "call_write".to_string(),
                backend_tool_name: "write_file".to_string(),
                backend_arguments: json!({"path": "a.txt"}),
            },
            ResponseRouting::ReplayOriginalTool,
        );
        ctx.suspend(SuspendTicket::from_invocation(invocation));

        assert!(ctx.tool_pending());
        // ReplayOriginalTool should use a dedicated frontend call id.
        assert!(
            call_id.starts_with("fc_"),
            "expected generated ID, got: {call_id}"
        );
        assert_ne!(call_id, "call_write");

        let invocation = ctx
            .tool
            .as_ref()
            .unwrap()
            .suspend_ticket
            .as_ref()
            .and_then(|ticket| ticket.invocation.as_ref())
            .expect("pending frontend invocation should exist");
        assert_eq!(invocation.call_id, call_id);
        assert_eq!(invocation.tool_name, "PermissionConfirm");
        assert!(matches!(
            invocation.routing,
            ResponseRouting::ReplayOriginalTool
        ));
        match &invocation.origin {
            InvocationOrigin::ToolCallIntercepted {
                backend_call_id,
                backend_tool_name,
                ..
            } => {
                assert_eq!(backend_call_id, "call_write");
                assert_eq!(backend_tool_name, "write_file");
            }
            other => panic!("expected ToolCallIntercepted, got: {other:?}"),
        }
    }

    #[test]
    fn test_suspend_invocation_without_tool_context_noop() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        let invocation = FrontendToolInvocation::new(
            "fc_noop",
            "PermissionConfirm",
            json!({}),
            InvocationOrigin::PluginInitiated {
                plugin_id: "agui_frontend_tools".to_string(),
            },
            ResponseRouting::UseAsToolResult,
        );
        ctx.suspend(SuspendTicket::from_invocation(invocation));
        assert!(!ctx.tool_pending());
    }

    #[test]
    fn test_set_clear_session_context() {
        let fix = TestFixture::new();
        let mut ctx = fix.step(vec![]);

        ctx.thread("Context 1");
        ctx.thread("Context 2");
        ctx.session_context = vec!["Only this".to_string()];

        assert_eq!(ctx.session_context.len(), 1);
        assert_eq!(ctx.session_context[0], "Only this");
    }
}
