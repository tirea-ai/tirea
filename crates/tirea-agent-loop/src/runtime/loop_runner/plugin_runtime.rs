use super::core::{clear_agent_inference_error, set_agent_inference_error};
use super::AgentLoopError;
use crate::contracts::runtime::behavior::{
    build_read_only_context_from_step, AgentBehavior, ReadOnlyContext,
};
use crate::contracts::runtime::action::Action;
use crate::contracts::runtime::phase::{Phase, StepContext};
use crate::contracts::runtime::state::{
    reduce_state_actions, AnyStateAction, CommutativeAction,
};
use crate::contracts::runtime::tool_call::ToolDescriptor;
use crate::contracts::RunContext;
use crate::contracts::ToolCallContext;
use crate::runtime::control::InferenceError;
use serde_json::Value;
use std::sync::Mutex;
use tirea_state::{DocCell, Patch, PatchExt, TrackedPatch};

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
    emit_agent_phase_with_options(phase, step, agent, doc, false).await
}

async fn emit_agent_phase_with_options(
    phase: Phase,
    step: &mut StepContext<'_>,
    agent: &dyn AgentBehavior,
    doc: &DocCell,
    _defer_commutative_state_actions: bool,
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
        let patches = reduce_state_actions(state_actions, &doc.snapshot(), "agent:phase")
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

fn take_step_pending_commutative_actions(step: &mut StepContext<'_>) -> Vec<CommutativeAction> {
    std::mem::take(&mut step.pending_commutative_actions)
}

fn merge_tracked_patches(patches: &[TrackedPatch], source: &str) -> Option<TrackedPatch> {
    let mut merged = Patch::new();
    for tracked in patches {
        merged.extend(tracked.patch().clone());
    }
    if merged.is_empty() {
        None
    } else {
        Some(TrackedPatch::new(merged).with_source(source.to_string()))
    }
}

fn reduce_commutative_actions_to_patch(
    base_snapshot: &Value,
    actions: Vec<CommutativeAction>,
    source: &str,
) -> Result<Option<TrackedPatch>, AgentLoopError> {
    if actions.is_empty() {
        return Ok(None);
    }

    let tracked = reduce_state_actions(
        actions
            .into_iter()
            .map(AnyStateAction::Commutative)
            .collect(),
        base_snapshot,
        source,
    )
    .map_err(|e| {
        AgentLoopError::StateError(format!("failed to reduce commutative state actions: {e}"))
    })?;
    Ok(merge_tracked_patches(&tracked, source))
}

fn finalize_step_pending_outputs(
    step: &mut StepContext<'_>,
    base_snapshot: &Value,
    defer_commutative_state_actions: bool,
) -> Result<Vec<TrackedPatch>, AgentLoopError> {
    let mut pending = take_step_pending_patches(step);
    let commutative_actions = take_step_pending_commutative_actions(step);

    if !defer_commutative_state_actions || commutative_actions.is_empty() {
        return Ok(pending);
    }

    if let Some(commutative_patch) =
        reduce_commutative_actions_to_patch(base_snapshot, commutative_actions, "agent_loop")?
    {
        for patch in &pending {
            let conflicts = patch.patch().conflicts_with(commutative_patch.patch());
            if let Some(conflict) = conflicts.first() {
                return Err(AgentLoopError::StateError(format!(
                    "conflicting phase state patches between '{}' and 'commutative_actions' at {}",
                    patch.source.as_deref().unwrap_or("agent"),
                    conflict.path
                )));
            }
        }
        pending.push(commutative_patch);
    }

    Ok(pending)
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
        emit_agent_phase_with_options(*phase, &mut step, agent.behavior(), &doc, true).await?;
    }
    let ctx_patch = step.ctx().take_patch();
    if !ctx_patch.patch().is_empty() {
        step.emit_patch(ctx_patch);
    }
    let output = extract(&mut step);
    let pending = finalize_step_pending_outputs(&mut step, &doc.snapshot(), true)?;
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
    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let set_error_patch = set_agent_inference_error(
        &state,
        InferenceError {
            error_type: error_type.to_string(),
            message,
        },
    )?;
    run_ctx.add_thread_patch(set_error_patch);

    let pending = emit_phase_block(
        Phase::AfterInference,
        run_ctx,
        tool_descriptors,
        agent,
        |_| {},
    )
    .await?;
    run_ctx.add_thread_patches(pending);

    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let clear_error_patch = clear_agent_inference_error(&state)?;
    run_ctx.add_thread_patch(clear_error_patch);

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
        if let Err(e) =
            emit_agent_phase_with_options(Phase::RunEnd, &mut step, agent.behavior(), &doc, true)
                .await
        {
            tracing::warn!(error = %e, "RunEnd phase validation failed");
        }
        let ctx_patch = step.ctx().take_patch();
        if !ctx_patch.patch().is_empty() {
            step.emit_patch(ctx_patch);
        }
        match finalize_step_pending_outputs(&mut step, &doc.snapshot(), true) {
            Ok(pending) => pending,
            Err(e) => {
                tracing::warn!(error = %e, "RunEnd phase commutative reduce failed");
                Vec::new()
            }
        }
    };
    run_ctx.add_thread_patches(pending);
}

/// Tool-level phase dispatch.
pub(super) async fn emit_tool_phase(
    phase: Phase,
    step: &mut StepContext<'_>,
    agent: Option<&dyn AgentBehavior>,
    doc: &DocCell,
    defer_commutative_state_actions: bool,
) -> Result<(), AgentLoopError> {
    if let Some(agent) = agent {
        if defer_commutative_state_actions {
            emit_agent_phase_with_options(phase, step, agent, doc, true).await
        } else {
            emit_agent_phase(phase, step, agent, doc).await
        }
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
        emit_agent_phase_with_options(*phase, &mut step, behavior, &doc, true).await?;
    }
    let ctx_patch = step.ctx().take_patch();
    if !ctx_patch.patch().is_empty() {
        step.emit_patch(ctx_patch);
    }
    let output = extract(&mut step);
    let pending = finalize_step_pending_outputs(&mut step, &doc.snapshot(), true)?;
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
