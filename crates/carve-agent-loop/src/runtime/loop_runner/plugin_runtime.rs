use super::core::{
    clear_agent_inference_error, set_agent_inference_error,
};
use super::AgentLoopError;
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::plugin::phase::{Phase, StepContext};
use crate::contracts::tool::ToolDescriptor;
use crate::contracts::RunContext;
use crate::contracts::ToolCallContext;
use crate::runtime::control::InferenceError;
use carve_state::{DocCell, TrackedPatch};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
struct PhaseMutationSnapshot {
    skip_inference: bool,
    tool_ids: Vec<String>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    tool_blocked: bool,
    tool_pending: bool,
    tool_pending_interaction_id: Option<String>,
    tool_has_result: bool,
}

fn phase_mutation_snapshot(step: &StepContext<'_>) -> PhaseMutationSnapshot {
    PhaseMutationSnapshot {
        skip_inference: step.skip_inference,
        tool_ids: step.tools.iter().map(|t| t.id.clone()).collect(),
        tool_call_id: step.tool.as_ref().map(|t| t.id.clone()),
        tool_name: step.tool.as_ref().map(|t| t.name.clone()),
        tool_blocked: step.tool_blocked(),
        tool_pending: step.tool_pending(),
        tool_pending_interaction_id: step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.as_ref().map(|i| i.id.clone())),
        tool_has_result: step.tool.as_ref().and_then(|t| t.result.as_ref()).is_some(),
    }
}

fn validate_phase_mutation(
    phase: Phase,
    plugin_id: &str,
    before: &PhaseMutationSnapshot,
    after: &PhaseMutationSnapshot,
) -> Result<(), AgentLoopError> {
    let policy = phase.policy();

    if before.tool_ids != after.tool_ids && !policy.allow_tool_filter_mutation {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool filtering outside BeforeInference ({phase})",
            plugin_id
        )));
    }

    if before.skip_inference != after.skip_inference && !policy.allow_skip_inference_mutation {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated skip_inference outside BeforeInference ({phase})",
            plugin_id
        )));
    }

    if before.tool_call_id != after.tool_call_id || before.tool_name != after.tool_name {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool identity in phase {phase}",
            plugin_id
        )));
    }

    let tool_gate_changed = before.tool_blocked != after.tool_blocked
        || before.tool_pending != after.tool_pending
        || before.tool_pending_interaction_id != after.tool_pending_interaction_id;
    if tool_gate_changed && !policy.allow_tool_gate_mutation {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool gate outside BeforeToolExecute ({phase})",
            plugin_id
        )));
    }

    if before.tool_has_result != after.tool_has_result {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool result in phase {phase}",
            plugin_id
        )));
    }

    Ok(())
}

pub(super) async fn emit_phase_checked(
    phase: Phase,
    step: &mut StepContext<'_>,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<(), AgentLoopError> {
    for plugin in plugins {
        let before = phase_mutation_snapshot(step);
        plugin.on_phase(phase, step).await;
        let after = phase_mutation_snapshot(step);
        validate_phase_mutation(phase, plugin.id(), &before, &after)?;
    }
    Ok(())
}

fn take_step_pending_patches(step: &mut StepContext<'_>) -> Vec<TrackedPatch> {
    std::mem::take(&mut step.pending_patches)
}

pub(super) async fn run_phase_block<R, Setup, Extract>(
    run_ctx: &RunContext,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
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
        "plugin:phase",
        &run_ctx.run_config,
        &pending_messages,
        None,
    );
    let mut step = StepContext::new(
        tool_call_ctx,
        run_ctx.thread_id(),
        run_ctx.messages(),
        tool_descriptors.to_vec(),
    );
    setup(&mut step);
    for phase in phases {
        emit_phase_checked(*phase, &mut step, plugins).await?;
    }
    let plugin_patch = step.ctx().take_patch();
    if !plugin_patch.patch().is_empty() {
        step.pending_patches.push(plugin_patch);
    }
    let output = extract(&mut step);
    let pending = take_step_pending_patches(&mut step);
    Ok((output, pending))
}

pub(super) async fn emit_phase_block<Setup>(
    phase: Phase,
    run_ctx: &RunContext,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    setup: Setup,
) -> Result<Vec<TrackedPatch>, AgentLoopError>
where
    Setup: FnOnce(&mut StepContext<'_>),
{
    let (_, pending) =
        run_phase_block(run_ctx, tool_descriptors, plugins, &[phase], setup, |_| ()).await?;
    Ok(pending)
}

pub(super) async fn emit_cleanup_phases_and_apply(
    run_ctx: &mut RunContext,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
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
    );
    run_ctx.add_thread_patch(set_error_patch);

    let pending = emit_phase_block(
        Phase::AfterInference,
        run_ctx,
        tool_descriptors,
        plugins,
        |_| {},
    )
    .await?;
    run_ctx.add_thread_patches(pending);

    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let clear_error_patch = clear_agent_inference_error(&state);
    run_ctx.add_thread_patch(clear_error_patch);

    let pending =
        emit_phase_block(Phase::StepEnd, run_ctx, tool_descriptors, plugins, |_| {}).await?;
    run_ctx.add_thread_patches(pending);
    Ok(())
}

pub(super) async fn emit_run_end_phase(
    run_ctx: &mut RunContext,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
) {
    let pending = {
        let current_state = match run_ctx.snapshot() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "RunEndPhase(RunEnd): failed to rebuild state");
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
            "plugin:run_end",
            &run_ctx.run_config,
            &pending_messages,
            None,
        );
        let mut step = StepContext::new(
            tool_call_ctx,
            run_ctx.thread_id(),
            run_ctx.messages(),
            tool_descriptors.to_vec(),
        );
        if let Err(e) = emit_phase_checked(Phase::RunEnd, &mut step, plugins).await {
            tracing::warn!(error = %e, "RunEndPhase(RunEnd) plugin phase validation failed");
        }
        let plugin_patch = step.ctx().take_patch();
        if !plugin_patch.patch().is_empty() {
            step.pending_patches.push(plugin_patch);
        }
        take_step_pending_patches(&mut step)
    };
    run_ctx.add_thread_patches(pending);
}
