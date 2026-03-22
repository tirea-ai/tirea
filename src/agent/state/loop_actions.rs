use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::contract::context_message::ContextMessage;
use crate::contract::inference::InferenceOverride;
use crate::state::{MergeStrategy, StateKey};

// ---------------------------------------------------------------------------
// Action specs
// ---------------------------------------------------------------------------

/// Action spec for injecting a context message into the prompt.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<AddContextMessage>(...)`.
/// Handled during `run_phase(BeforeInference)` — the handler applies throttle logic
/// and writes accepted messages to [`AccumulatedContextMessages`].
pub struct AddContextMessage;

impl crate::model::ScheduledActionSpec for AddContextMessage {
    const KEY: &'static str = "runtime.add_context_message";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = ContextMessage;
}

/// Action spec for per-inference parameter overrides.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<SetInferenceOverride>(...)`.
/// Handled during `run_phase(BeforeInference)` — the handler merges payloads with
/// last-wins semantics per field into [`AccumulatedOverrides`].
pub struct SetInferenceOverride;

impl crate::model::ScheduledActionSpec for SetInferenceOverride {
    const KEY: &'static str = "runtime.set_inference_override";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = InferenceOverride;
}

/// Action spec for excluding a specific tool from the current inference step.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<ExcludeTool>(...)`.
/// Handled during `run_phase(BeforeInference)` — the handler accumulates tool IDs
/// into [`AccumulatedToolExclusions`].
pub struct ExcludeTool;

impl crate::model::ScheduledActionSpec for ExcludeTool {
    const KEY: &'static str = "runtime.exclude_tool";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = String;
}

/// Action spec for restricting tools to an explicit allow-list for the current inference step.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<IncludeOnlyTools>(...)`.
/// Handled during `run_phase(BeforeInference)` — the handler merges allow-lists
/// into [`AccumulatedToolInclusions`].
pub struct IncludeOnlyTools;

impl crate::model::ScheduledActionSpec for IncludeOnlyTools {
    const KEY: &'static str = "runtime.include_only_tools";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = Vec<String>;
}

// ---------------------------------------------------------------------------
// Accumulator state keys — written by action handlers, read by the orchestrator
// ---------------------------------------------------------------------------

/// Accumulated inference overrides for the current step.
///
/// The `SetInferenceOverride` handler merges each payload into this accumulator.
/// The orchestrator reads and clears it after `run_phase(BeforeInference)`.
pub struct AccumulatedOverrides;

/// Update for [`AccumulatedOverrides`].
pub enum AccumulatedOverridesUpdate {
    /// Merge an override (last-wins per field).
    Merge(InferenceOverride),
    /// Clear the accumulator (at step start).
    Clear,
}

impl StateKey for AccumulatedOverrides {
    const KEY: &'static str = "__runtime.accumulated_overrides";
    const MERGE: MergeStrategy = MergeStrategy::Commutative;

    type Value = Option<InferenceOverride>;
    type Update = AccumulatedOverridesUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            AccumulatedOverridesUpdate::Merge(ovr) => {
                if let Some(existing) = value.as_mut() {
                    existing.merge(ovr);
                } else {
                    *value = Some(ovr);
                }
            }
            AccumulatedOverridesUpdate::Clear => {
                *value = None;
            }
        }
    }
}

/// Accumulated context messages that passed throttle for the current step.
///
/// The `AddContextMessage` handler applies throttle logic and pushes accepted
/// messages here. The orchestrator reads and clears after `run_phase(BeforeInference)`.
pub struct AccumulatedContextMessages;

/// Update for [`AccumulatedContextMessages`].
pub enum AccumulatedContextMessagesUpdate {
    /// Push an accepted context message.
    Push(ContextMessage),
    /// Clear the accumulator (at step start).
    Clear,
}

impl StateKey for AccumulatedContextMessages {
    const KEY: &'static str = "__runtime.accumulated_context_messages";
    const MERGE: MergeStrategy = MergeStrategy::Commutative;

    type Value = Vec<ContextMessage>;
    type Update = AccumulatedContextMessagesUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            AccumulatedContextMessagesUpdate::Push(msg) => value.push(msg),
            AccumulatedContextMessagesUpdate::Clear => value.clear(),
        }
    }
}

/// Accumulated tool exclusion IDs for the current step.
///
/// The `ExcludeTool` handler pushes each tool ID here.
/// The orchestrator reads and clears after `run_phase(BeforeInference)`.
pub struct AccumulatedToolExclusions;

/// Update for [`AccumulatedToolExclusions`].
pub enum AccumulatedToolExclusionsUpdate {
    /// Add a tool ID to exclude.
    Add(String),
    /// Clear the accumulator (at step start).
    Clear,
}

impl StateKey for AccumulatedToolExclusions {
    const KEY: &'static str = "__runtime.accumulated_tool_exclusions";
    const MERGE: MergeStrategy = MergeStrategy::Commutative;

    type Value = HashSet<String>;
    type Update = AccumulatedToolExclusionsUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            AccumulatedToolExclusionsUpdate::Add(id) => {
                value.insert(id);
            }
            AccumulatedToolExclusionsUpdate::Clear => value.clear(),
        }
    }
}

/// Accumulated tool inclusion allow-list for the current step.
///
/// The `IncludeOnlyTools` handler extends this with each allow-list union.
/// The orchestrator reads and clears after `run_phase(BeforeInference)`.
pub struct AccumulatedToolInclusions;

/// Update for [`AccumulatedToolInclusions`].
pub enum AccumulatedToolInclusionsUpdate {
    /// Extend the allow-list with additional tool IDs.
    Extend(Vec<String>),
    /// Clear the accumulator (at step start).
    Clear,
}

/// The value is `None` when no `IncludeOnlyTools` action has been scheduled,
/// and `Some(set)` when at least one has been processed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInclusionSet(pub Option<HashSet<String>>);

impl StateKey for AccumulatedToolInclusions {
    const KEY: &'static str = "__runtime.accumulated_tool_inclusions";
    const MERGE: MergeStrategy = MergeStrategy::Commutative;

    type Value = ToolInclusionSet;
    type Update = AccumulatedToolInclusionsUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            AccumulatedToolInclusionsUpdate::Extend(ids) => {
                let set = value.0.get_or_insert_with(HashSet::new);
                set.extend(ids);
            }
            AccumulatedToolInclusionsUpdate::Clear => {
                value.0 = None;
            }
        }
    }
}
