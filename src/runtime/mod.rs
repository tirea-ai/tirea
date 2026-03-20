mod app;
mod context;
mod engine;
mod handlers;
mod registry;
mod reports;

pub use app::AppRuntime;
pub use context::PhaseContext;
pub use engine::PhaseRuntime;
pub use handlers::{PhaseHook, TypedEffectHandler, TypedScheduledActionHandler};
pub use reports::{
    DEFAULT_MAX_PHASE_ROUNDS, EffectDispatchReport, PhaseRunReport, SubmitCommandReport,
};

pub(crate) use handlers::{
    EffectHandlerArc, PhaseHookArc, ScheduledActionHandlerArc, TypedEffectAdapter,
    TypedScheduledActionAdapter,
};
