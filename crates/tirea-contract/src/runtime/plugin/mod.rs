pub mod agent;
pub mod phase;

pub use agent::{
    build_read_only_context_from_step, AgentBehavior, NoOpBehavior, ReadOnlyContext,
};
pub use phase::{
    reduce_state_actions, Action, AfterInferenceContext, AfterToolExecuteContext, AnyStateAction,
    BeforeInferenceContext, BeforeToolExecuteContext, CommutativeAction, Extensions, Phase,
    PhaseContext, PhasePolicy, RunAction, RunEndContext, RunStartContext, StateScope, StateSpec,
    StepContext, StepEndContext, StepOutcome, StepStartContext, SuspendTicket, ToolCallAction,
    ToolGate,
};
