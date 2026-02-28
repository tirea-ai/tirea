use super::core::ext::{FlowControl, InferenceContext, LLMResponse, MessagingContext, ToolGate};
use super::extensions::Extensions;
use super::state_spec::{AnyStateAction, CommutativeAction};
use super::{RunAction, StepOutcome, SuspendTicket, ToolCallAction};
use crate::runtime::tool_call::ToolCallContext;
use crate::runtime::tool_call::{ToolDescriptor, ToolResult};
use crate::thread::Message;
use crate::RunConfig;
use serde_json::Value;
use std::sync::Arc;
use tirea_state::{State, TireaResult, TrackedPatch};

/// Step context - mutable state passed through all phases.
///
/// This is the primary interface for plugins to interact with the agent loop.
/// All phase-specific data lives in the [`Extensions`] type map. Core extension
/// types (`InferenceContext`, `ToolGate`, `MessagingContext`, `FlowControl`,
/// `LLMResponse`) are defined in the [`core::ext`](super::core::ext) module.
pub struct StepContext<'a> {
    // === Execution Context ===
    ctx: ToolCallContext<'a>,

    // === Identity ===
    thread_id: &'a str,

    // === Thread Messages ===
    messages: &'a [Arc<Message>],

    // === Pending State Changes ===
    pub pending_patches: Vec<TrackedPatch>,
    pub pending_commutative_actions: Vec<CommutativeAction>,
    pub pending_state_actions: Vec<AnyStateAction>,

    // === Extensions ===
    pub extensions: Extensions,
}

impl<'a> StepContext<'a> {
    /// Create a new step context.
    ///
    /// `tools` are stored in the `InferenceContext` extension.
    pub fn new(
        ctx: ToolCallContext<'a>,
        thread_id: &'a str,
        messages: &'a [Arc<Message>],
        tools: Vec<ToolDescriptor>,
    ) -> Self {
        let mut extensions = Extensions::new();
        extensions.insert(InferenceContext {
            tools,
            ..Default::default()
        });
        Self {
            ctx,
            thread_id,
            messages,
            pending_patches: Vec::new(),
            pending_commutative_actions: Vec::new(),
            pending_state_actions: Vec::new(),
            extensions,
        }
    }

    // =========================================================================
    // Execution context access
    // =========================================================================

    pub fn ctx(&self) -> &ToolCallContext<'a> {
        &self.ctx
    }

    pub fn thread_id(&self) -> &str {
        self.thread_id
    }

    pub fn messages(&self) -> &[Arc<Message>] {
        self.messages
    }

    pub fn state_of<T: State>(&self) -> T::Ref<'_> {
        self.ctx.state_of::<T>()
    }

    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        self.ctx.state::<T>(path)
    }

    pub fn run_config(&self) -> &RunConfig {
        self.ctx.run_config()
    }

    pub fn config_state<T: State>(&self) -> TireaResult<T::Ref<'_>> {
        self.ctx.config_state::<T>()
    }

    pub fn snapshot(&self) -> Value {
        self.ctx.snapshot()
    }

    pub fn snapshot_of<T: State>(&self) -> TireaResult<T> {
        self.ctx.snapshot_of::<T>()
    }

    pub fn snapshot_at<T: State>(&self, path: &str) -> TireaResult<T> {
        self.ctx.snapshot_at::<T>(path)
    }

    /// Reset step-specific state for a new step.
    ///
    /// Preserves `InferenceContext.tools` across resets.
    pub fn reset(&mut self) {
        let tools = self
            .extensions
            .get::<InferenceContext>()
            .map(|inf| inf.tools.clone())
            .unwrap_or_default();
        self.extensions.clear();
        self.extensions.insert(InferenceContext {
            tools,
            ..Default::default()
        });
        self.pending_patches.clear();
        self.pending_commutative_actions.clear();
        self.pending_state_actions.clear();
    }

    // =========================================================================
    // Context Injection — delegates to Extensions
    // =========================================================================

    /// Add system context (appended to system prompt) [Position 1].
    pub fn system(&mut self, content: impl Into<String>) {
        self.extensions
            .get_or_default::<InferenceContext>()
            .system_context
            .push(content.into());
    }

    /// Add session context message (before user messages) [Position 2].
    pub fn thread(&mut self, content: impl Into<String>) {
        self.extensions
            .get_or_default::<InferenceContext>()
            .session_context
            .push(content.into());
    }

    /// Add system reminder (after tool result) [Position 7].
    pub fn reminder(&mut self, content: impl Into<String>) {
        self.extensions
            .get_or_default::<MessagingContext>()
            .reminders
            .push(content.into());
    }

    /// Append a user message to be injected after tool execution.
    pub fn user_message(&mut self, content: impl Into<String>) {
        self.extensions
            .get_or_default::<MessagingContext>()
            .user_messages
            .push(content.into());
    }

    // =========================================================================
    // Tool Filtering — delegates to InferenceContext.tools
    // =========================================================================

    /// Exclude a tool by ID.
    pub fn exclude(&mut self, tool_id: &str) {
        if let Some(inf) = self.extensions.get_mut::<InferenceContext>() {
            inf.tools.retain(|t| t.id != tool_id);
        }
    }

    /// Include only specified tools.
    pub fn include_only(&mut self, tool_ids: &[&str]) {
        if let Some(inf) = self.extensions.get_mut::<InferenceContext>() {
            inf.tools.retain(|t| tool_ids.contains(&t.id.as_str()));
        }
    }

    // =========================================================================
    // Tool Control — delegates to ToolGate extension
    // =========================================================================

    pub fn tool_name(&self) -> Option<&str> {
        self.extensions
            .get::<ToolGate>()
            .map(|g| g.name.as_str())
    }

    pub fn tool_call_id(&self) -> Option<&str> {
        self.extensions.get::<ToolGate>().map(|g| g.id.as_str())
    }

    pub fn tool_idempotency_key(&self) -> Option<&str> {
        self.tool_call_id()
    }

    pub fn tool_args(&self) -> Option<&Value> {
        self.extensions.get::<ToolGate>().map(|g| &g.args)
    }

    pub fn tool_result(&self) -> Option<&ToolResult> {
        self.extensions
            .get::<ToolGate>()
            .and_then(|g| g.result.as_ref())
    }

    pub fn tool_blocked(&self) -> bool {
        self.extensions
            .get::<ToolGate>()
            .map(|g| g.blocked)
            .unwrap_or(false)
    }

    pub fn tool_pending(&self) -> bool {
        self.extensions
            .get::<ToolGate>()
            .map(|g| g.pending)
            .unwrap_or(false)
    }

    /// Mark the current tool as explicitly allowed.
    pub fn allow(&mut self) {
        if let Some(gate) = self.extensions.get_mut::<ToolGate>() {
            gate.blocked = false;
            gate.block_reason = None;
            gate.pending = false;
            gate.suspend_ticket = None;
        }
    }

    /// Mark the current tool as blocked with a reason.
    pub fn block(&mut self, reason: impl Into<String>) {
        if let Some(gate) = self.extensions.get_mut::<ToolGate>() {
            gate.blocked = true;
            gate.block_reason = Some(reason.into());
            gate.pending = false;
            gate.suspend_ticket = None;
        }
    }

    /// Mark the current tool as suspended with a ticket.
    pub fn suspend(&mut self, ticket: SuspendTicket) {
        if let Some(gate) = self.extensions.get_mut::<ToolGate>() {
            gate.blocked = false;
            gate.block_reason = None;
            gate.pending = true;
            gate.suspend_ticket = Some(ticket);
        }
    }

    /// Set tool result.
    pub fn set_tool_result(&mut self, result: ToolResult) {
        if let Some(gate) = self.extensions.get_mut::<ToolGate>() {
            gate.result = Some(result);
        }
    }

    /// Set run-level action.
    pub fn set_run_action(&mut self, action: RunAction) {
        self.extensions
            .get_or_default::<FlowControl>()
            .run_action = Some(action);
    }

    /// Emit a state patch side effect.
    pub fn emit_patch(&mut self, patch: TrackedPatch) {
        self.pending_patches.push(patch);
    }

    /// Emit a commutative state action side effect.
    pub fn emit_commutative_action(&mut self, action: CommutativeAction) {
        self.pending_commutative_actions.push(action);
    }

    /// Emit a state action to be reduced into a patch after the phase completes.
    pub fn emit_state_action(&mut self, action: AnyStateAction) {
        self.pending_state_actions.push(action);
    }

    /// Effective run-level action for current step.
    pub fn run_action(&self) -> RunAction {
        self.extensions
            .get::<FlowControl>()
            .and_then(|fc| fc.run_action.clone())
            .unwrap_or(RunAction::Continue)
    }

    /// Current tool action derived from tool gate state.
    pub fn tool_action(&self) -> ToolCallAction {
        if let Some(gate) = self.extensions.get::<ToolGate>() {
            if gate.blocked {
                return ToolCallAction::Block {
                    reason: gate.block_reason.clone().unwrap_or_default(),
                };
            }
            if gate.pending {
                if let Some(ticket) = gate.suspend_ticket.as_ref() {
                    return ToolCallAction::suspend(ticket.clone());
                }
                return ToolCallAction::Block {
                    reason: "invalid pending tool state: missing suspend ticket".to_string(),
                };
            }
        }
        ToolCallAction::Proceed
    }

    // =========================================================================
    // Step Outcome
    // =========================================================================

    /// Get the step outcome based on current state.
    pub fn result(&self) -> StepOutcome {
        if let Some(gate) = self.extensions.get::<ToolGate>() {
            if gate.pending {
                if let Some(ticket) = gate.suspend_ticket.as_ref() {
                    return StepOutcome::Pending(ticket.clone());
                }
                return StepOutcome::Continue;
            }
        }

        if let Some(llm) = self.extensions.get::<LLMResponse>() {
            if llm.result.tool_calls.is_empty() && !llm.result.text.is_empty() {
                return StepOutcome::Complete;
            }
        }

        StepOutcome::Continue
    }
}
