use super::AgentLoopError;
use crate::contracts::runtime::phase::StepContext;
use crate::contracts::runtime::{Interaction, InteractionResponse};
use crate::contracts::state::AgentState;
use crate::contracts::state::{Message, MessageMetadata};
use crate::contracts::tool::Tool;
use crate::runtime::control::{
    InferenceError, LoopControlExt, LoopControlState, LOOP_CONTROL_STATE_PATH,
};
use carve_state::{StateContext, TrackedPatch};

/// Skills state path — matches `carve_agent_extension_skills::SKILLS_STATE_PATH`.
const SKILLS_STATE_PATH: &str = "skills";

/// Interaction outbox path — matches `carve_agent_extension_interaction::INTERACTION_OUTBOX_PATH`.
const INTERACTION_OUTBOX_PATH: &str = "interaction_outbox";

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct ThreadMutationBatch {
    pub(super) messages: Vec<Message>,
    pub(super) patches: Vec<TrackedPatch>,
}

impl ThreadMutationBatch {
    pub(super) fn with_message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    pub(super) fn with_messages(mut self, messages: impl IntoIterator<Item = Message>) -> Self {
        self.messages.extend(messages);
        self
    }

    pub(super) fn with_patch(mut self, patch: TrackedPatch) -> Self {
        self.patches.push(patch);
        self
    }

    pub(super) fn with_patches(mut self, patches: impl IntoIterator<Item = TrackedPatch>) -> Self {
        self.patches.extend(patches);
        self
    }
}

pub(super) fn reduce_thread_mutations(
    thread: AgentState,
    batch: ThreadMutationBatch,
) -> AgentState {
    let mut thread = thread;
    if !batch.patches.is_empty() {
        thread = thread.with_patches(batch.patches);
    }
    if !batch.messages.is_empty() {
        thread = thread.with_messages(batch.messages);
    }
    thread
}

pub(super) fn apply_pending_patches(thread: AgentState, pending: Vec<TrackedPatch>) -> AgentState {
    reduce_thread_mutations(thread, ThreadMutationBatch::default().with_patches(pending))
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

    for msg in &step.thread.messages {
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
) -> TrackedPatch {
    let ctx = StateContext::new(state);
    let lc = ctx.state::<LoopControlState>(LOOP_CONTROL_STATE_PATH);
    lc.set_pending_interaction(Some(interaction));
    ctx.take_tracked_patch("agent_loop")
}

pub(super) fn clear_agent_pending_interaction(state: &Value) -> TrackedPatch {
    let ctx = StateContext::new(state);
    let lc = ctx.state::<LoopControlState>(LOOP_CONTROL_STATE_PATH);
    lc.pending_interaction_none();
    ctx.take_tracked_patch("agent_loop")
}

pub(super) fn pending_interaction_from_thread(thread: &AgentState) -> Option<Interaction> {
    thread.pending_interaction()
}

pub(super) fn set_agent_inference_error(state: &Value, error: InferenceError) -> TrackedPatch {
    let ctx = StateContext::new(state);
    let lc = ctx.state::<LoopControlState>(LOOP_CONTROL_STATE_PATH);
    lc.set_inference_error(Some(error));
    ctx.take_tracked_patch("agent_loop")
}

pub(super) fn clear_agent_inference_error(state: &Value) -> TrackedPatch {
    let ctx = StateContext::new(state);
    let lc = ctx.state::<LoopControlState>(LOOP_CONTROL_STATE_PATH);
    lc.inference_error_none();
    ctx.take_tracked_patch("agent_loop")
}

#[derive(Default)]
pub(super) struct AgentOutboxDrain {
    pub(super) interaction_resolutions: Vec<InteractionResponse>,
    pub(super) replay_tool_calls: Vec<crate::contracts::state::ToolCall>,
}

pub(super) fn drain_agent_outbox(
    mut thread: AgentState,
    _call_id: &str,
) -> Result<(AgentState, AgentOutboxDrain), AgentLoopError> {
    let state = thread
        .rebuild_state()
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
        Some(raw) => serde_json::from_value::<Vec<crate::contracts::state::ToolCall>>(raw)
            .map_err(|e| {
                AgentLoopError::StateError(format!(
                    "failed to parse interaction_outbox.replay_tool_calls: {e}"
                ))
            })?,
        None => Vec::new(),
    };

    if interaction_resolutions.is_empty() && replay_tool_calls.is_empty() {
        return Ok((thread, AgentOutboxDrain::default()));
    }

    // Clear consumed fields via raw patch (no type dependency on InteractionOutbox)
    let outbox_path = carve_state::Path::root().key(INTERACTION_OUTBOX_PATH);
    let mut clear_patch = carve_state::Patch::new();
    if !interaction_resolutions.is_empty() {
        clear_patch = clear_patch.with_op(carve_state::Op::set(
            outbox_path.clone().key("interaction_resolutions"),
            serde_json::json!([]),
        ));
    }
    if !replay_tool_calls.is_empty() {
        clear_patch = clear_patch.with_op(carve_state::Op::set(
            outbox_path.key("replay_tool_calls"),
            serde_json::json!([]),
        ));
    }
    if !clear_patch.is_empty() {
        let patch = TrackedPatch::new(clear_patch).with_source("agent_loop");
        thread = reduce_thread_mutations(thread, ThreadMutationBatch::default().with_patch(patch));
    }

    Ok((
        thread,
        AgentOutboxDrain {
            interaction_resolutions,
            replay_tool_calls,
        },
    ))
}

pub(super) fn drain_agent_append_user_messages(
    mut thread: AgentState,
    results: &[super::ToolExecutionResult],
    metadata: Option<&MessageMetadata>,
) -> Result<(AgentState, usize), AgentLoopError> {
    let state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    let queued_by_call = state
        .get(SKILLS_STATE_PATH)
        .and_then(|s| s.get("append_user_messages"))
        .cloned()
        .and_then(|v| serde_json::from_value::<HashMap<String, Vec<String>>>(v).ok())
        .unwrap_or_default();

    if queued_by_call.is_empty() {
        return Ok((thread, 0));
    }

    let mut appended = 0usize;
    let mut seen_call_ids = std::collections::HashSet::new();
    let mut mutations = ThreadMutationBatch::default();

    for result in results {
        let call_id = result.execution.call.id.as_str();
        if !seen_call_ids.insert(call_id.to_string()) {
            continue;
        }
        let Some(messages) = queued_by_call.get(call_id) else {
            continue;
        };
        for text in messages.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let mut msg = Message::user(text.to_string());
            if let Some(meta) = metadata {
                msg.metadata = Some(meta.clone());
            }
            mutations = mutations.with_message(msg);
            appended += 1;
        }
    }

    let mut stale_keys: Vec<&String> = queued_by_call
        .keys()
        .filter(|key| !seen_call_ids.contains(*key))
        .collect();
    stale_keys.sort();
    for key in stale_keys {
        if let Some(messages) = queued_by_call.get(key) {
            for text in messages.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                let mut msg = Message::user(text.to_string());
                if let Some(meta) = metadata {
                    msg.metadata = Some(meta.clone());
                }
                mutations = mutations.with_message(msg);
                appended += 1;
            }
        }
    }

    let clear_patch = TrackedPatch::new(carve_state::Patch::with_ops(vec![carve_state::Op::set(
        carve_state::Path::root()
            .key(SKILLS_STATE_PATH)
            .key("append_user_messages"),
        serde_json::Value::Object(Default::default()),
    )]))
    .with_source("agent_loop");
    if !clear_patch.patch().is_empty() {
        mutations = mutations.with_patch(clear_patch);
    }

    if appended == 0 && mutations.patches.is_empty() {
        return Ok((thread, 0));
    }
    thread = reduce_thread_mutations(thread, mutations);
    Ok((thread, appended))
}
