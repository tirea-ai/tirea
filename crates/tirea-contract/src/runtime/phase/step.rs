use crate::runtime::extensions::Extensions;
use crate::runtime::inference::{InferenceContext, LLMResponse};
use crate::runtime::run::FlowControl;
use crate::runtime::state::AnyStateAction;
use crate::runtime::tool_call::gate::ToolGate;
use crate::runtime::tool_call::{ToolCallContext, ToolDescriptor, ToolResult};
use crate::thread::Message;
use crate::RunConfig;
use serde_json::Value;
use std::sync::Arc;
use tirea_state::{State, TireaResult, TrackedPatch};

use super::types::{RunAction, StepOutcome, ToolCallAction};

/// Step context - mutable state passed through all phases.
///
/// This is the primary interface for plugins to interact with the agent loop.
/// All phase-specific data lives in the [`Extensions`] type map. Core extension
/// types (`InferenceContext`, `ToolGate`, `MessagingContext`, `FlowControl`,
/// `LLMResponse`) are defined in dedicated domain modules.
pub struct StepContext<'a> {
    // === Execution Context ===
    ctx: ToolCallContext<'a>,

    // === Identity ===
    thread_id: &'a str,

    // === Thread Messages ===
    messages: &'a [Arc<Message>],

    // === Pending State Changes ===
    pub pending_patches: Vec<TrackedPatch>,
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
        self.pending_state_actions.clear();
    }

    // =========================================================================
    // Tool Control — read-only accessors for ToolGate extension
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

    /// Emit a state patch side effect.
    pub fn emit_patch(&mut self, patch: TrackedPatch) {
        self.pending_patches.push(patch);
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
            if let Ok(result) = &llm.outcome {
                if result.tool_calls.is_empty() && !result.text.is_empty() {
                    return StepOutcome::Complete;
                }
            }
        }

        StepOutcome::Continue
    }
}
