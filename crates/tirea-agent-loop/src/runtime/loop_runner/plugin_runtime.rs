use super::AgentLoopError;
use crate::contracts::runtime::behavior::{
    build_read_only_context_from_step, AgentBehavior, ReadOnlyContext,
};
use crate::contracts::runtime::action::Action;
use crate::contracts::runtime::inference::{InferenceError, LLMResponse};
use crate::contracts::runtime::phase::{Phase, StepContext};
use crate::contracts::runtime::state::{reduce_state_actions, ScopeContext};
use crate::contracts::runtime::tool_call::ToolDescriptor;
use crate::contracts::RunContext;
use crate::contracts::ToolCallContext;
use std::sync::Mutex;
use tirea_state::{DocCell, TrackedPatch};

// =========================================================================
// Agent-based dispatch (declarative model: ReadOnlyContext → Vec<Action>)
// =========================================================================

/// Dispatch a single phase hook on an [`AgentBehavior`].
///
/// Returns the actions emitted by the behavior for the given phase.
async fn dispatch_agent_phase<'a>(
    agent: &dyn AgentBehavior,
    phase: Phase,
    ctx: &ReadOnlyContext<'a>,
) -> Vec<Box<dyn Action>> {
    match phase {
        Phase::RunStart => agent.run_start(ctx).await,
        Phase::StepStart => agent.step_start(ctx).await,
        Phase::BeforeInference => agent.before_inference(ctx).await,
        Phase::AfterInference => agent.after_inference(ctx).await,
        Phase::BeforeToolExecute => agent.before_tool_execute(ctx).await,
        Phase::AfterToolExecute => agent.after_tool_execute(ctx).await,
        Phase::StepEnd => agent.step_end(ctx).await,
        Phase::RunEnd => agent.run_end(ctx).await,
    }
}

/// Emit a single phase using the [`AgentBehavior`] declarative model.
///
/// Builds a [`ReadOnlyContext`], calls the agent hook, validates and applies
/// the returned actions to the mutable `StepContext`.
pub(super) async fn emit_agent_phase(
    phase: Phase,
    step: &mut StepContext<'_>,
    agent: &dyn AgentBehavior,
    doc: &DocCell,
) -> Result<(), AgentLoopError> {
    let ctx = build_read_only_context_from_step(phase, step, doc);
    let actions = dispatch_agent_phase(agent, phase, &ctx).await;
    for action in &actions {
        action
            .validate(phase)
            .map_err(AgentLoopError::StateError)?;
    }
    for action in actions {
        action.apply(step);
    }
    // Reduce any pending_state_actions that were added by EmitStatePatch actions.
    let state_actions = std::mem::take(&mut step.pending_state_actions);
    if !state_actions.is_empty() {
        // Tool phases carry a call_id for ToolCall-scoped routing.
        let scope_ctx = match phase {
            Phase::BeforeToolExecute | Phase::AfterToolExecute => step
                .tool_call_id()
                .map(ScopeContext::for_call)
                .unwrap_or_else(ScopeContext::run),
            _ => ScopeContext::run(),
        };
        let patches =
            reduce_state_actions(state_actions, &doc.snapshot(), "agent:phase", &scope_ctx)
                .map_err(|e| {
                    AgentLoopError::StateError(format!(
                        "failed to reduce pending state actions: {e}"
                    ))
                })?;
        for p in patches {
            step.emit_patch(p);
        }
    }
    Ok(())
}

// =========================================================================
// Shared helpers
// =========================================================================

fn take_step_pending_patches(step: &mut StepContext<'_>) -> Vec<TrackedPatch> {
    std::mem::take(&mut step.pending_patches)
}

/// Multi-phase block dispatch.
pub(super) async fn run_phase_block<R, Setup, Extract>(
    run_ctx: &RunContext,
    tool_descriptors: &[ToolDescriptor],
    agent: &dyn super::Agent,
    phases: &[Phase],
    setup: Setup,
    extract: Extract,
) -> Result<(R, Vec<TrackedPatch>), AgentLoopError>
where
    Setup: FnOnce(&mut StepContext<'_>),
    Extract: FnOnce(&mut StepContext<'_>) -> R,
{
    let current_state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let doc = DocCell::new(current_state);
    let ops = Mutex::new(Vec::new());
    let pending_messages = Mutex::new(Vec::new());
    let tool_call_ctx = ToolCallContext::new(
        &doc,
        &ops,
        "phase",
        "agent:phase",
        &run_ctx.run_config,
        &pending_messages,
        tirea_contract::runtime::activity::NoOpActivityManager::arc(),
    );
    let mut step = StepContext::new(
        tool_call_ctx,
        run_ctx.thread_id(),
        run_ctx.messages(),
        tool_descriptors.to_vec(),
    );
    setup(&mut step);
    for phase in phases {
        emit_agent_phase(*phase, &mut step, agent.behavior(), &doc).await?;
    }
    let ctx_patch = step.ctx().take_patch();
    if !ctx_patch.patch().is_empty() {
        step.emit_patch(ctx_patch);
    }
    let output = extract(&mut step);
    let pending = take_step_pending_patches(&mut step);
    Ok((output, pending))
}

/// Single-phase block dispatch (no extract value).
pub(super) async fn emit_phase_block<Setup>(
    phase: Phase,
    run_ctx: &RunContext,
    tool_descriptors: &[ToolDescriptor],
    agent: &dyn super::Agent,
    setup: Setup,
) -> Result<Vec<TrackedPatch>, AgentLoopError>
where
    Setup: FnOnce(&mut StepContext<'_>),
{
    let (_, pending) =
        run_phase_block(run_ctx, tool_descriptors, agent, &[phase], setup, |_| ()).await?;
    Ok(pending)
}

/// Cleanup dispatch (after LLM error).
pub(super) async fn emit_cleanup_phases(
    run_ctx: &mut RunContext,
    tool_descriptors: &[ToolDescriptor],
    agent: &dyn super::Agent,
    error_type: &'static str,
    message: String,
) -> Result<(), AgentLoopError> {
    let error = InferenceError {
        error_type: error_type.to_string(),
        message,
    };

    let pending = emit_phase_block(
        Phase::AfterInference,
        run_ctx,
        tool_descriptors,
        agent,
        |step| {
            step.extensions.insert(LLMResponse::error(error));
        },
    )
    .await?;
    run_ctx.add_thread_patches(pending);

    let pending =
        emit_phase_block(Phase::StepEnd, run_ctx, tool_descriptors, agent, |_| {}).await?;
    run_ctx.add_thread_patches(pending);
    Ok(())
}

/// Run-end phase dispatch.
pub(super) async fn emit_run_end_phase(
    run_ctx: &mut RunContext,
    tool_descriptors: &[ToolDescriptor],
    agent: &dyn super::Agent,
) {
    let pending = {
        let current_state = match run_ctx.snapshot() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "RunEnd: failed to rebuild state");
                return;
            }
        };
        let doc = DocCell::new(current_state);
        let ops = Mutex::new(Vec::new());
        let pending_messages = Mutex::new(Vec::new());
        let tool_call_ctx = ToolCallContext::new(
            &doc,
            &ops,
            "phase",
            "agent:run_end",
            &run_ctx.run_config,
            &pending_messages,
            tirea_contract::runtime::activity::NoOpActivityManager::arc(),
        );
        let mut step = StepContext::new(
            tool_call_ctx,
            run_ctx.thread_id(),
            run_ctx.messages(),
            tool_descriptors.to_vec(),
        );
        if let Err(e) = emit_agent_phase(Phase::RunEnd, &mut step, agent.behavior(), &doc).await {
            tracing::warn!(error = %e, "RunEnd phase validation failed");
        }
        let ctx_patch = step.ctx().take_patch();
        if !ctx_patch.patch().is_empty() {
            step.emit_patch(ctx_patch);
        }
        take_step_pending_patches(&mut step)
    };
    run_ctx.add_thread_patches(pending);
}

/// Tool-level phase dispatch.
pub(super) async fn emit_tool_phase(
    phase: Phase,
    step: &mut StepContext<'_>,
    agent: Option<&dyn AgentBehavior>,
    doc: &DocCell,
) -> Result<(), AgentLoopError> {
    if let Some(agent) = agent {
        emit_agent_phase(phase, step, agent, doc).await
    } else {
        Ok(())
    }
}

// =========================================================================
// Behavior-only block helper (used by tool_exec.rs public APIs)
// =========================================================================

pub(super) async fn behavior_run_phase_block<R, Setup, Extract>(
    run_ctx: &RunContext,
    tool_descriptors: &[ToolDescriptor],
    behavior: &dyn AgentBehavior,
    phases: &[Phase],
    setup: Setup,
    extract: Extract,
) -> Result<(R, Vec<TrackedPatch>), AgentLoopError>
where
    Setup: FnOnce(&mut StepContext<'_>),
    Extract: FnOnce(&mut StepContext<'_>) -> R,
{
    let current_state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let doc = DocCell::new(current_state);
    let ops = Mutex::new(Vec::new());
    let pending_messages = Mutex::new(Vec::new());
    let tool_call_ctx = ToolCallContext::new(
        &doc,
        &ops,
        "phase",
        "behavior:phase",
        &run_ctx.run_config,
        &pending_messages,
        tirea_contract::runtime::activity::NoOpActivityManager::arc(),
    );
    let mut step = StepContext::new(
        tool_call_ctx,
        run_ctx.thread_id(),
        run_ctx.messages(),
        tool_descriptors.to_vec(),
    );
    setup(&mut step);
    for phase in phases {
        emit_agent_phase(*phase, &mut step, behavior, &doc).await?;
    }
    let ctx_patch = step.ctx().take_patch();
    if !ctx_patch.patch().is_empty() {
        step.emit_patch(ctx_patch);
    }
    let output = extract(&mut step);
    let pending = take_step_pending_patches(&mut step);
    Ok((output, pending))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::runtime::behavior::NoOpBehavior;
    use tirea_contract::testing::TestSystemContext as AddSystemContext;
    use crate::contracts::testing::TestFixture;
    use async_trait::async_trait;
    use tirea_state::DocCell;

    struct TestActionBehavior;

    #[async_trait]
    impl AgentBehavior for TestActionBehavior {
        fn id(&self) -> &str {
            "test_action"
        }

        async fn before_inference(
            &self,
            _ctx: &ReadOnlyContext<'_>,
        ) -> Vec<Box<dyn Action>> {
            vec![Box::new(AddSystemContext("injected by action".into()))]
        }
    }

    #[tokio::test]
    async fn emit_agent_phase_validates_and_applies_actions() {
        use crate::contracts::runtime::inference::InferenceContext;

        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);
        let doc = DocCell::new(serde_json::json!({}));

        emit_agent_phase(Phase::BeforeInference, &mut step, &TestActionBehavior, &doc)
            .await
            .expect("actions should be validated and applied");

        let inf = step.extensions.get::<InferenceContext>().unwrap();
        assert_eq!(inf.system_context, vec!["injected by action"]);
    }

    #[tokio::test]
    async fn emit_agent_phase_noop_behavior_succeeds() {
        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);
        let doc = DocCell::new(serde_json::json!({}));

        emit_agent_phase(Phase::RunStart, &mut step, &NoOpBehavior, &doc)
            .await
            .expect("noop behavior should succeed");
    }
}
