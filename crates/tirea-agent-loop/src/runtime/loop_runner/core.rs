use super::AgentLoopError;
use crate::contracts::plugin::phase::StepContext;
use crate::contracts::thread::{Message, MessageMetadata, Role};
use crate::contracts::tool::Tool;
use crate::contracts::RunContext;
use crate::contracts::{FrontendToolInvocation, Interaction, InteractionResponse};
use crate::runtime::control::{InferenceError, LoopControlState};
use tirea_state::{DocCell, StateContext, TireaError, TrackedPatch};

/// Skills state path — matches `SkillState::PATH` (avoids type dependency).
const SKILLS_STATE_PATH: &str = "skills";

/// Interaction outbox path — matches `InteractionOutbox::PATH` (avoids type dependency).
const INTERACTION_OUTBOX_PATH: &str = "interaction_outbox";

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

fn is_pending_approval_placeholder(msg: &Message) -> bool {
    msg.role == Role::Tool
        && msg
            .content
            .contains("is awaiting approval. Execution paused.")
}

pub(super) fn build_messages(step: &StepContext<'_>, system_prompt: &str) -> Vec<Message> {
    let mut messages = Vec::new();

    let system = if step.system_context.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, step.system_context.join("\n"))
    };

    if !system.is_empty() {
        messages.push(Message::system(system));
    }

    for ctx in &step.session_context {
        messages.push(Message::system(ctx.clone()));
    }

    // Collect all tool_call IDs issued by the assistant so we can filter
    // orphaned tool results (e.g. from intercepted pseudo-tool invocations
    // like PermissionConfirm whose call IDs the LLM never issued).
    let known_tool_call_ids: HashSet<&str> = step
        .messages()
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .map(|tc| tc.id.as_str())
        .collect();

    // When a frontend tool pending placeholder is followed by a real tool result
    // for the same call_id, keep only the real result in inference context.
    // This preserves append-only persisted history while avoiding stale
    // "awaiting approval" text from biasing subsequent model turns.
    let mut pending_placeholder_ids = HashSet::new();
    let mut resolved_result_ids = HashSet::new();
    for msg in step.messages() {
        let Some(tc_id) = msg.tool_call_id.as_deref() else {
            continue;
        };
        if !known_tool_call_ids.contains(tc_id) {
            continue;
        }
        if is_pending_approval_placeholder(msg) {
            pending_placeholder_ids.insert(tc_id.to_string());
        } else if msg.role == Role::Tool {
            resolved_result_ids.insert(tc_id.to_string());
        }
    }
    let superseded_pending_ids: HashSet<String> = pending_placeholder_ids
        .intersection(&resolved_result_ids)
        .cloned()
        .collect();

    for msg in step.messages() {
        if msg.role == Role::Tool {
            if let Some(ref tc_id) = msg.tool_call_id {
                if !known_tool_call_ids.contains(tc_id.as_str()) {
                    continue;
                }
                if superseded_pending_ids.contains(tc_id) && is_pending_approval_placeholder(msg) {
                    continue;
                }
            }
        }
        messages.push((**msg).clone());
    }

    messages
}

pub(super) type InferenceInputs = (Vec<Message>, Vec<String>, bool);

pub(super) fn inference_inputs_from_step(
    step: &mut StepContext<'_>,
    system_prompt: &str,
) -> InferenceInputs {
    let messages = build_messages(step, system_prompt);
    let filtered_tools = step
        .tools
        .iter()
        .map(|td| td.id.clone())
        .collect::<Vec<_>>();
    let skip_inference = step.skip_inference;
    (messages, filtered_tools, skip_inference)
}

pub(super) fn build_request_for_filtered_tools(
    messages: &[Message],
    tools: &HashMap<String, Arc<dyn Tool>>,
    filtered_tools: &[String],
) -> genai::chat::ChatRequest {
    let filtered_tool_refs: Vec<&dyn Tool> = tools
        .values()
        .filter(|t| filtered_tools.contains(&t.descriptor().id))
        .map(|t| t.as_ref())
        .collect();
    crate::engine::convert::build_request(messages, &filtered_tool_refs)
}

pub(super) fn set_agent_pending_interaction(
    state: &Value,
    interaction: Interaction,
    frontend_invocation: Option<FrontendToolInvocation>,
) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let lc = ctx.state_of::<LoopControlState>();
    lc.set_pending_interaction(Some(interaction)).map_err(|e| {
        AgentLoopError::StateError(format!(
            "failed to set loop_control.pending_interaction: {e}"
        ))
    })?;
    lc.set_pending_frontend_invocation(frontend_invocation)
        .map_err(|e| {
            AgentLoopError::StateError(format!(
                "failed to set loop_control.pending_frontend_invocation: {e}"
            ))
        })?;
    Ok(ctx.take_tracked_patch("agent_loop"))
}

pub(super) fn clear_agent_pending_interaction(
    state: &Value,
) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let lc = ctx.state_of::<LoopControlState>();
    match lc.pending_interaction_none() {
        Ok(()) | Err(TireaError::PathNotFound { .. }) => {}
        Err(e) => {
            return Err(AgentLoopError::StateError(format!(
                "failed to clear loop_control.pending_interaction: {e}"
            )))
        }
    }
    match lc.pending_frontend_invocation_none() {
        Ok(()) | Err(TireaError::PathNotFound { .. }) => {}
        Err(e) => {
            return Err(AgentLoopError::StateError(format!(
                "failed to clear loop_control.pending_frontend_invocation: {e}"
            )))
        }
    }
    Ok(ctx.take_tracked_patch("agent_loop"))
}

pub(super) fn pending_interaction_from_ctx(run_ctx: &RunContext) -> Option<Interaction> {
    run_ctx.pending_interaction()
}

pub(super) fn pending_frontend_invocation_from_ctx(
    run_ctx: &RunContext,
) -> Option<crate::contracts::FrontendToolInvocation> {
    run_ctx.pending_frontend_invocation()
}

pub(super) fn set_agent_inference_error(
    state: &Value,
    error: InferenceError,
) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let lc = ctx.state_of::<LoopControlState>();
    lc.set_inference_error(Some(error)).map_err(|e| {
        AgentLoopError::StateError(format!("failed to set loop_control.inference_error: {e}"))
    })?;
    Ok(ctx.take_tracked_patch("agent_loop"))
}

pub(super) fn clear_agent_inference_error(state: &Value) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let lc = ctx.state_of::<LoopControlState>();
    match lc.inference_error_none() {
        Ok(()) | Err(TireaError::PathNotFound { .. }) => {}
        Err(e) => {
            return Err(AgentLoopError::StateError(format!(
                "failed to clear loop_control.inference_error: {e}"
            )))
        }
    }
    Ok(ctx.take_tracked_patch("agent_loop"))
}

#[derive(Default)]
pub(super) struct AgentOutboxDrain {
    pub(super) interaction_resolutions: Vec<InteractionResponse>,
    pub(super) replay_tool_calls: Vec<crate::contracts::thread::ToolCall>,
}

pub(super) fn drain_agent_outbox(
    run_ctx: &mut RunContext,
    _call_id: &str,
) -> Result<AgentOutboxDrain, AgentLoopError> {
    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    let interaction_resolutions = match state
        .get(INTERACTION_OUTBOX_PATH)
        .and_then(|outbox| outbox.get("interaction_resolutions"))
        .cloned()
    {
        Some(raw) => serde_json::from_value::<Vec<InteractionResponse>>(raw).map_err(|e| {
            AgentLoopError::StateError(format!(
                "failed to parse interaction_outbox.interaction_resolutions: {e}"
            ))
        })?,
        None => Vec::new(),
    };
    let replay_tool_calls = match state
        .get(INTERACTION_OUTBOX_PATH)
        .and_then(|outbox| outbox.get("replay_tool_calls"))
        .cloned()
    {
        Some(raw) => serde_json::from_value::<Vec<crate::contracts::thread::ToolCall>>(raw)
            .map_err(|e| {
                AgentLoopError::StateError(format!(
                    "failed to parse interaction_outbox.replay_tool_calls: {e}"
                ))
            })?,
        None => Vec::new(),
    };

    if interaction_resolutions.is_empty() && replay_tool_calls.is_empty() {
        return Ok(AgentOutboxDrain::default());
    }

    // Clear consumed fields via raw patch (no type dependency on InteractionOutbox)
    let outbox_path = tirea_state::Path::root().key(INTERACTION_OUTBOX_PATH);
    let mut clear_patch = tirea_state::Patch::new();
    if !interaction_resolutions.is_empty() {
        clear_patch = clear_patch.with_op(tirea_state::Op::set(
            outbox_path.clone().key("interaction_resolutions"),
            serde_json::json!([]),
        ));
    }
    if !replay_tool_calls.is_empty() {
        clear_patch = clear_patch.with_op(tirea_state::Op::set(
            outbox_path.key("replay_tool_calls"),
            serde_json::json!([]),
        ));
    }
    if !clear_patch.is_empty() {
        let patch = TrackedPatch::new(clear_patch).with_source("agent_loop");
        run_ctx.add_thread_patch(patch);
    }

    Ok(AgentOutboxDrain {
        interaction_resolutions,
        replay_tool_calls,
    })
}

pub(super) fn drain_agent_append_user_messages(
    run_ctx: &mut RunContext,
    results: &[super::ToolExecutionResult],
    metadata: Option<&MessageMetadata>,
) -> Result<usize, AgentLoopError> {
    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    let queued_by_call = state
        .get(SKILLS_STATE_PATH)
        .and_then(|s| s.get("append_user_messages"))
        .cloned()
        .and_then(|v| serde_json::from_value::<HashMap<String, Vec<String>>>(v).ok())
        .unwrap_or_default();

    if queued_by_call.is_empty() {
        return Ok(0);
    }

    let mut appended = 0usize;
    let mut seen_call_ids = std::collections::HashSet::new();
    let mut messages = Vec::new();
    let mut patches = Vec::new();

    for result in results {
        let call_id = result.execution.call.id.as_str();
        if !seen_call_ids.insert(call_id.to_string()) {
            continue;
        }
        let Some(queued_messages) = queued_by_call.get(call_id) else {
            continue;
        };
        for text in queued_messages
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let mut msg = Message::user(text.to_string());
            if let Some(meta) = metadata {
                msg.metadata = Some(meta.clone());
            }
            messages.push(Arc::new(msg));
            appended += 1;
        }
    }

    let mut stale_keys: Vec<&String> = queued_by_call
        .keys()
        .filter(|key| !seen_call_ids.contains(*key))
        .collect();
    stale_keys.sort();
    for key in stale_keys {
        if let Some(queued_messages) = queued_by_call.get(key) {
            for text in queued_messages
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                let mut msg = Message::user(text.to_string());
                if let Some(meta) = metadata {
                    msg.metadata = Some(meta.clone());
                }
                messages.push(Arc::new(msg));
                appended += 1;
            }
        }
    }

    let clear_patch = TrackedPatch::new(tirea_state::Patch::with_ops(vec![tirea_state::Op::set(
        tirea_state::Path::root()
            .key(SKILLS_STATE_PATH)
            .key("append_user_messages"),
        serde_json::Value::Object(Default::default()),
    )]))
    .with_source("agent_loop");
    if !clear_patch.patch().is_empty() {
        patches.push(clear_patch);
    }

    if appended == 0 && patches.is_empty() {
        return Ok(0);
    }
    run_ctx.add_messages(messages);
    run_ctx.add_thread_patches(patches);
    Ok(appended)
}
