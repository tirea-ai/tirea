use super::core::ext::{
    FlowControl, InferenceContext, LLMResponse, MessagingContext, ToolGate,
};
use super::{Phase, RunAction, StepContext, SuspendTicket, ToolCallAction};
use crate::runtime::llm::StreamResult;
use crate::runtime::run::TerminationReason;
use crate::runtime::tool_call::ToolResult;
use crate::runtime::ToolCallResume;
use crate::thread::Message;
use crate::RunConfig;
use serde_json::Value;
use std::sync::Arc;
use tirea_state::State;

/// Shared read access available to all phase contexts.
pub trait PhaseContext {
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

macro_rules! impl_phase_context {
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

        impl<'s, 'a> PhaseContext for $name<'s, 'a> {
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
impl_phase_context!(RunStartContext, Phase::RunStart);

pub struct StepStartContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_phase_context!(StepStartContext, Phase::StepStart);

pub struct BeforeInferenceContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_phase_context!(BeforeInferenceContext, Phase::BeforeInference);

impl<'s, 'a> BeforeInferenceContext<'s, 'a> {
    /// Append a system context line.
    pub fn add_system_context(&mut self, text: impl Into<String>) {
        self.step
            .extensions
            .get_or_default::<InferenceContext>()
            .system_context
            .push(text.into());
    }

    /// Append a session message.
    pub fn add_session_message(&mut self, text: impl Into<String>) {
        self.step
            .extensions
            .get_or_default::<InferenceContext>()
            .session_context
            .push(text.into());
    }

    /// Exclude tool by id.
    pub fn exclude_tool(&mut self, tool_id: &str) {
        if let Some(inf) = self.step.extensions.get_mut::<InferenceContext>() {
            inf.tools.retain(|t| t.id != tool_id);
        }
    }

    /// Keep only listed tools.
    pub fn include_only(&mut self, tool_ids: &[&str]) {
        if let Some(inf) = self.step.extensions.get_mut::<InferenceContext>() {
            inf.tools.retain(|t| tool_ids.contains(&t.id.as_str()));
        }
    }

    /// Terminate current run as behavior-requested before inference.
    pub fn terminate_behavior_requested(&mut self) {
        self.step
            .extensions
            .get_or_default::<FlowControl>()
            .run_action = Some(RunAction::Terminate(TerminationReason::BehaviorRequested));
    }

    /// Request run termination with a specific reason.
    pub fn request_termination(&mut self, reason: TerminationReason) {
        self.step
            .extensions
            .get_or_default::<FlowControl>()
            .run_action = Some(RunAction::Terminate(reason));
    }
}

pub struct AfterInferenceContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_phase_context!(AfterInferenceContext, Phase::AfterInference);

impl<'s, 'a> AfterInferenceContext<'s, 'a> {
    pub fn response_opt(&self) -> Option<&StreamResult> {
        self.step
            .extensions
            .get::<LLMResponse>()
            .map(|r| &r.result)
    }

    pub fn response(&self) -> &StreamResult {
        &self
            .step
            .extensions
            .get::<LLMResponse>()
            .expect("AfterInferenceContext.response() requires response to be set")
            .result
    }

    /// Request run termination with a specific reason after inference has completed.
    pub fn request_termination(&mut self, reason: TerminationReason) {
        self.step
            .extensions
            .get_or_default::<FlowControl>()
            .run_action = Some(RunAction::Terminate(reason));
    }
}

pub struct BeforeToolExecuteContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_phase_context!(BeforeToolExecuteContext, Phase::BeforeToolExecute);

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

    /// Resume payload attached to current tool call, if present.
    pub fn resume_input(&self) -> Option<ToolCallResume> {
        let gate = self.step.extensions.get::<ToolGate>()?;
        self.step.ctx().resume_input_for(&gate.id).ok().flatten()
    }

    pub fn decision(&self) -> ToolCallAction {
        self.step.tool_action()
    }

    pub fn set_decision(&mut self, decision: ToolCallAction) {
        if let Some(gate) = self.step.extensions.get_mut::<ToolGate>() {
            match decision {
                ToolCallAction::Proceed => {
                    gate.blocked = false;
                    gate.block_reason = None;
                    gate.pending = false;
                    gate.suspend_ticket = None;
                }
                ToolCallAction::Suspend(ticket) => {
                    gate.blocked = false;
                    gate.block_reason = None;
                    gate.pending = true;
                    gate.suspend_ticket = Some(*ticket);
                }
                ToolCallAction::Block { reason } => {
                    gate.blocked = true;
                    gate.block_reason = Some(reason);
                    gate.pending = false;
                    gate.suspend_ticket = None;
                }
            }
        }
    }

    pub fn block(&mut self, reason: impl Into<String>) {
        if let Some(gate) = self.step.extensions.get_mut::<ToolGate>() {
            gate.blocked = true;
            gate.block_reason = Some(reason.into());
            gate.pending = false;
            gate.suspend_ticket = None;
        }
    }

    /// Explicitly allow tool execution.
    ///
    /// This clears any previous block/suspend state set by earlier plugins.
    pub fn allow(&mut self) {
        if let Some(gate) = self.step.extensions.get_mut::<ToolGate>() {
            gate.blocked = false;
            gate.block_reason = None;
            gate.pending = false;
            gate.suspend_ticket = None;
        }
    }

    /// Override current call result directly from plugin logic.
    ///
    /// Useful for resumed frontend interactions where the external payload
    /// should become the tool result without executing a backend tool.
    pub fn set_tool_result(&mut self, result: ToolResult) {
        if let Some(gate) = self.step.extensions.get_mut::<ToolGate>() {
            gate.result = Some(result);
        }
    }

    pub fn suspend(&mut self, ticket: SuspendTicket) {
        if let Some(gate) = self.step.extensions.get_mut::<ToolGate>() {
            gate.blocked = false;
            gate.block_reason = None;
            gate.pending = true;
            gate.suspend_ticket = Some(ticket);
        }
    }
}

pub struct AfterToolExecuteContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_phase_context!(AfterToolExecuteContext, Phase::AfterToolExecute);

impl<'s, 'a> AfterToolExecuteContext<'s, 'a> {
    pub fn tool_name(&self) -> Option<&str> {
        self.step.tool_name()
    }

    pub fn tool_call_id(&self) -> Option<&str> {
        self.step.tool_call_id()
    }

    pub fn tool_result(&self) -> &ToolResult {
        self.step
            .extensions
            .get::<ToolGate>()
            .and_then(|g| g.result.as_ref())
            .expect("AfterToolExecuteContext.tool_result() requires tool result")
    }

    pub fn add_system_reminder(&mut self, text: impl Into<String>) {
        self.step
            .extensions
            .get_or_default::<MessagingContext>()
            .reminders
            .push(text.into());
    }
}

pub struct StepEndContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_phase_context!(StepEndContext, Phase::StepEnd);

pub struct RunEndContext<'s, 'a> {
    step: &'s mut StepContext<'a>,
}
impl_phase_context!(RunEndContext, Phase::RunEnd);
