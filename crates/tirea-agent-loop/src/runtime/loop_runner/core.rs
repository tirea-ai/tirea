use super::AgentLoopError;
use crate::contracts::plugin::phase::StepContext;
use crate::contracts::runtime::state_paths::{
    INFERENCE_ERROR_STATE_PATH, RESUME_DECISIONS_STATE_PATH, SKILLS_STATE_PATH,
    SUSPENDED_TOOL_CALLS_STATE_PATH,
};
use crate::contracts::thread::{Message, MessageMetadata, Role};
use crate::contracts::tool::Tool;
use crate::contracts::RunAction;
use crate::contracts::RunContext;
use crate::contracts::SuspendedCall;
use crate::runtime::control::{
    InferenceError, InferenceErrorState, ResumeDecision, ResumeDecisionsState,
    SuspendedToolCallsState,
};
use tirea_state::{DocCell, StateContext, TireaError, TrackedPatch};

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

pub(super) type InferenceInputs = (Vec<Message>, Vec<String>, RunAction);

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
    let run_action = step.run_action();
    (messages, filtered_tools, run_action)
}

pub(super) fn build_request_for_filtered_tools(
    messages: &[Message],
    tools: &HashMap<String, Arc<dyn Tool>>,
    filtered_tools: &[String],
) -> genai::chat::ChatRequest {
    let filtered: HashSet<&str> = filtered_tools.iter().map(String::as_str).collect();
    let filtered_tool_refs: Vec<&dyn Tool> = tools
        .values()
        .filter(|t| filtered.contains(t.descriptor().id.as_str()))
        .map(|t| t.as_ref())
        .collect();
    crate::engine::convert::build_request(messages, &filtered_tool_refs)
}

/// Write suspended calls to internal state.
pub(super) fn set_agent_suspended_calls(
    state: &Value,
    calls: Vec<SuspendedCall>,
) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let suspended_state = ctx.state_of::<SuspendedToolCallsState>();

    let map: HashMap<String, SuspendedCall> =
        calls.into_iter().map(|c| (c.call_id.clone(), c)).collect();
    suspended_state.set_calls(map).map_err(|e| {
        AgentLoopError::StateError(format!(
            "failed to set {SUSPENDED_TOOL_CALLS_STATE_PATH}.calls: {e}"
        ))
    })?;
    Ok(ctx.take_tracked_patch("agent_loop"))
}

/// Clear all suspended calls.
pub(super) fn clear_all_suspended_calls(state: &Value) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let suspended_state = ctx.state_of::<SuspendedToolCallsState>();

    match suspended_state.set_calls(HashMap::new()) {
        Ok(()) | Err(TireaError::PathNotFound { .. }) => {}
        Err(e) => {
            return Err(AgentLoopError::StateError(format!(
                "failed to clear {SUSPENDED_TOOL_CALLS_STATE_PATH}.calls: {e}"
            )))
        }
    }
    Ok(ctx.take_tracked_patch("agent_loop"))
}

/// Clear one suspended call.
pub(super) fn clear_suspended_call(
    state: &Value,
    call_id: &str,
) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let suspended_state = ctx.state_of::<SuspendedToolCallsState>();
    let mut suspended = state
        .get(SUSPENDED_TOOL_CALLS_STATE_PATH)
        .and_then(|s| s.get("calls"))
        .cloned()
        .and_then(|raw| serde_json::from_value::<HashMap<String, SuspendedCall>>(raw).ok())
        .unwrap_or_default();

    if suspended.remove(call_id).is_none() {
        return Ok(ctx.take_tracked_patch("agent_loop"));
    }

    suspended_state.set_calls(suspended).map_err(|e| {
        AgentLoopError::StateError(format!(
            "failed to set {SUSPENDED_TOOL_CALLS_STATE_PATH}.calls: {e}"
        ))
    })?;
    Ok(ctx.take_tracked_patch("agent_loop"))
}

#[allow(dead_code)]
pub(super) fn suspended_calls_from_ctx(run_ctx: &RunContext) -> HashMap<String, SuspendedCall> {
    run_ctx.suspended_calls()
}

pub(super) fn set_agent_inference_error(
    state: &Value,
    error: InferenceError,
) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let inference = ctx.state_of::<InferenceErrorState>();
    inference.set_error(Some(error)).map_err(|e| {
        AgentLoopError::StateError(format!(
            "failed to set {INFERENCE_ERROR_STATE_PATH}.error: {e}"
        ))
    })?;
    Ok(ctx.take_tracked_patch("agent_loop"))
}

pub(super) fn resume_decisions_from_ctx(run_ctx: &RunContext) -> HashMap<String, ResumeDecision> {
    run_ctx
        .snapshot()
        .ok()
        .and_then(|state| {
            state
                .get(RESUME_DECISIONS_STATE_PATH)
                .and_then(|v| v.get("calls"))
                .cloned()
                .and_then(|raw| serde_json::from_value::<HashMap<String, ResumeDecision>>(raw).ok())
        })
        .unwrap_or_default()
}

pub(super) fn upsert_resume_decision(
    state: &Value,
    call_id: &str,
    decision: ResumeDecision,
) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let mailbox = ctx.state_of::<ResumeDecisionsState>();
    let mut decisions = mailbox.calls().ok().unwrap_or_default();
    decisions.insert(call_id.to_string(), decision);
    mailbox.set_calls(decisions).map_err(|e| {
        AgentLoopError::StateError(format!(
            "failed to set {RESUME_DECISIONS_STATE_PATH}.calls: {e}"
        ))
    })?;
    Ok(ctx.take_tracked_patch("agent_loop"))
}

pub(super) fn clear_resume_decisions(
    state: &Value,
    call_ids: &[String],
) -> Result<TrackedPatch, AgentLoopError> {
    if call_ids.is_empty() {
        return Ok(TrackedPatch::new(tirea_state::Patch::new()).with_source("agent_loop"));
    }

    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let mailbox = ctx.state_of::<ResumeDecisionsState>();
    let mut decisions = mailbox.calls().ok().unwrap_or_default();
    let mut changed = false;
    for call_id in call_ids {
        if decisions.remove(call_id).is_some() {
            changed = true;
        }
    }
    if !changed {
        return Ok(ctx.take_tracked_patch("agent_loop"));
    }
    mailbox.set_calls(decisions).map_err(|e| {
        AgentLoopError::StateError(format!(
            "failed to set {RESUME_DECISIONS_STATE_PATH}.calls: {e}"
        ))
    })?;
    Ok(ctx.take_tracked_patch("agent_loop"))
}

pub(super) fn clear_agent_inference_error(state: &Value) -> Result<TrackedPatch, AgentLoopError> {
    let doc = DocCell::new(state.clone());
    let ctx = StateContext::new(&doc);
    let inference = ctx.state_of::<InferenceErrorState>();
    match inference.error_none() {
        Ok(()) | Err(TireaError::PathNotFound { .. }) => {}
        Err(e) => {
            return Err(AgentLoopError::StateError(format!(
                "failed to clear {INFERENCE_ERROR_STATE_PATH}.error: {e}"
            )))
        }
    }
    Ok(ctx.take_tracked_patch("agent_loop"))
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
