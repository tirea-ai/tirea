//! Loop-consumed actions: inference overrides, context messages, and tool filters.

use crate::contract::message::{Message, Role};
use crate::model::{PendingScheduledActions, ScheduledActionQueueUpdate};
use crate::state::MutationBatch;

/// Consume `SetInferenceOverride` actions from the pending queue.
///
/// Loop-consumed action: no handler registered, EXECUTE skips it.
/// Multiple overrides are merged with last-wins semantics per field.
pub(super) fn consume_inference_overrides(
    store: &crate::state::StateStore,
) -> Result<Option<crate::contract::inference::InferenceOverride>, crate::error::StateError> {
    use super::super::state::SetInferenceOverride;
    use crate::model::ScheduledActionSpec;

    let pending = store.read::<PendingScheduledActions>().unwrap_or_default();

    let matching: Vec<_> = pending
        .iter()
        .filter(|e| e.action.key == SetInferenceOverride::KEY)
        .collect();

    if matching.is_empty() {
        return Ok(None);
    }

    let mut merged = crate::contract::inference::InferenceOverride::default();
    let mut ids = Vec::new();
    for envelope in matching {
        let payload = SetInferenceOverride::decode_payload(envelope.action.payload.clone())?;
        merged.merge(payload);
        ids.push(envelope.id);
    }

    // Dequeue consumed actions
    let mut patch = MutationBatch::new();
    for id in ids {
        patch.update::<PendingScheduledActions>(ScheduledActionQueueUpdate::Remove { id });
    }
    store.commit(patch)?;

    if merged.is_empty() {
        Ok(None)
    } else {
        Ok(Some(merged))
    }
}

/// Consume `AddContextMessage` actions from the pending queue with throttle filtering.
///
/// Reads `ContextThrottleState` to enforce cooldown rules:
/// - `cooldown_turns == 0`: always inject
/// - Content hash changed since last injection: inject
/// - Steps since last injection >= cooldown_turns: inject
/// - Otherwise: skip (throttled)
///
/// All matching actions are dequeued regardless of throttle outcome.
pub(super) fn consume_context_messages(
    store: &crate::state::StateStore,
    current_step: usize,
) -> Result<Vec<crate::contract::context_message::ContextMessage>, crate::error::StateError> {
    use super::super::state::{AddContextMessage, ContextThrottleState, ContextThrottleUpdate};
    use crate::model::ScheduledActionSpec;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let pending = store.read::<PendingScheduledActions>().unwrap_or_default();

    let matching: Vec<_> = pending
        .iter()
        .filter(|e| e.action.key == AddContextMessage::KEY)
        .collect();

    if matching.is_empty() {
        return Ok(vec![]);
    }

    // Decode all payloads and collect action IDs
    let mut candidates = Vec::new();
    let mut action_ids = Vec::new();
    for envelope in matching {
        let payload = AddContextMessage::decode_payload(envelope.action.payload.clone())?;
        candidates.push(payload);
        action_ids.push(envelope.id);
    }

    // Dequeue all matching actions (consumed regardless of throttle)
    let mut patch = MutationBatch::new();
    for id in &action_ids {
        patch.update::<PendingScheduledActions>(ScheduledActionQueueUpdate::Remove { id: *id });
    }
    store.commit(patch)?;

    // Apply throttle filtering
    let throttle_state = store.read::<ContextThrottleState>().unwrap_or_default();

    let mut accepted = Vec::new();
    let mut throttle_updates = Vec::new();

    for msg in candidates {
        let content_hash = {
            let mut hasher = DefaultHasher::new();
            // Hash the serialized content for change detection
            if let Ok(json) = serde_json::to_string(&msg.content) {
                json.hash(&mut hasher);
            }
            hasher.finish()
        };

        let should_inject = if msg.cooldown_turns == 0 {
            true
        } else {
            match throttle_state.entries.get(&msg.key) {
                None => true,
                Some(entry) => {
                    entry.content_hash != content_hash
                        || current_step.saturating_sub(entry.last_step)
                            >= msg.cooldown_turns as usize
                }
            }
        };

        if should_inject {
            throttle_updates.push(ContextThrottleUpdate::Injected {
                key: msg.key.clone(),
                step: current_step,
                content_hash,
            });
            accepted.push(msg);
        }
    }

    // Update throttle state
    if !throttle_updates.is_empty() {
        let mut patch = MutationBatch::new();
        for update in throttle_updates {
            patch.update::<ContextThrottleState>(update);
        }
        store.commit(patch)?;
    }

    Ok(accepted)
}

/// Consume `ExcludeTool` and `IncludeOnlyTools` actions, then filter tool descriptors.
///
/// - All `ExcludeTool` payloads are collected; matching tool IDs are removed.
/// - If any `IncludeOnlyTools` payloads exist, their union forms an allow-list;
///   only tools whose IDs appear in the allow-list are kept.
/// - Exclusions are applied after inclusion filtering.
pub(super) fn apply_tool_filter_actions(
    store: &crate::state::StateStore,
    tools: &mut Vec<crate::contract::tool::ToolDescriptor>,
) -> Result<(), crate::error::StateError> {
    use super::super::state::{ExcludeTool, IncludeOnlyTools};
    use crate::model::ScheduledActionSpec;
    use std::collections::HashSet;

    let pending = store.read::<PendingScheduledActions>().unwrap_or_default();

    // Collect ExcludeTool actions
    let exclude_matching: Vec<_> = pending
        .iter()
        .filter(|e| e.action.key == ExcludeTool::KEY)
        .collect();

    let mut exclude_ids: HashSet<String> = HashSet::new();
    let mut action_ids: Vec<u64> = Vec::new();

    for envelope in &exclude_matching {
        let payload = ExcludeTool::decode_payload(envelope.action.payload.clone())?;
        exclude_ids.insert(payload);
        action_ids.push(envelope.id);
    }

    // Collect IncludeOnlyTools actions
    let include_matching: Vec<_> = pending
        .iter()
        .filter(|e| e.action.key == IncludeOnlyTools::KEY)
        .collect();

    let mut include_ids: Option<HashSet<String>> = None;

    for envelope in &include_matching {
        let payload = IncludeOnlyTools::decode_payload(envelope.action.payload.clone())?;
        let set = include_ids.get_or_insert_with(HashSet::new);
        set.extend(payload);
        action_ids.push(envelope.id);
    }

    // Dequeue all consumed actions
    if !action_ids.is_empty() {
        let mut patch = MutationBatch::new();
        for id in action_ids {
            patch.update::<PendingScheduledActions>(ScheduledActionQueueUpdate::Remove { id });
        }
        store.commit(patch)?;
    }

    // Apply include-only filter first
    if let Some(ref allowed) = include_ids {
        tools.retain(|t| allowed.contains(&t.id));
    }

    // Apply exclusions
    if !exclude_ids.is_empty() {
        tools.retain(|t| !exclude_ids.contains(&t.id));
    }

    Ok(())
}

/// Insert context messages into the message list at their declared target positions.
pub(super) fn apply_context_messages(
    messages: &mut Vec<Message>,
    context_messages: Vec<crate::contract::context_message::ContextMessage>,
    has_system_prompt: bool,
) {
    use crate::contract::context_message::ContextMessageTarget;

    let mut system = Vec::new();
    let mut session = Vec::new();
    let mut conversation = Vec::new();
    let mut suffix = Vec::new();

    for entry in context_messages {
        let msg = Message {
            id: Some(crate::contract::message::gen_message_id()),
            role: entry.role,
            content: entry.content,
            tool_calls: None,
            tool_call_id: None,
            visibility: entry.visibility,
            metadata: None,
        };
        match entry.target {
            ContextMessageTarget::System => system.push(msg),
            ContextMessageTarget::Session => session.push(msg),
            ContextMessageTarget::Conversation => conversation.push(msg),
            ContextMessageTarget::SuffixSystem => suffix.push(msg),
        }
    }

    // System: insert after base system prompt
    let system_insert_pos = usize::from(has_system_prompt);
    for (offset, msg) in system.into_iter().enumerate() {
        messages.insert(system_insert_pos + offset, msg);
    }

    // Session: insert after all system-role messages
    let session_insert_pos = messages
        .iter()
        .take_while(|m| m.role == Role::System)
        .count();
    for (offset, msg) in session.into_iter().enumerate() {
        messages.insert(session_insert_pos + offset, msg);
    }

    // Conversation: insert after system messages, before history
    let conversation_insert_pos = messages
        .iter()
        .take_while(|m| m.role == Role::System)
        .count();
    for (offset, msg) in conversation.into_iter().enumerate() {
        messages.insert(conversation_insert_pos + offset, msg);
    }

    // Suffix: append at end
    messages.extend(suffix);
}
