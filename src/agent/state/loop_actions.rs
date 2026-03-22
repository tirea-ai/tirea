use crate::contract::inference::InferenceOverride;

/// Action spec for injecting a context message into the prompt.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<AddContextMessage>(...)`.
/// The loop runner consumes all matching actions, groups by target, and inserts into
/// the message list at the appropriate positions before building the `InferenceRequest`.
pub struct AddContextMessage;

impl crate::model::ScheduledActionSpec for AddContextMessage {
    const KEY: &'static str = "runtime.add_context_message";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = crate::contract::context_message::ContextMessage;
}

/// Action spec for per-inference parameter overrides.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<SetInferenceOverride>(...)`.
/// The loop runner consumes all matching actions before building the `InferenceRequest`,
/// merging payloads with last-wins semantics per field. No handler registration needed.
pub struct SetInferenceOverride;

impl crate::model::ScheduledActionSpec for SetInferenceOverride {
    const KEY: &'static str = "runtime.set_inference_override";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = InferenceOverride;
}

/// Action spec for excluding a specific tool from the current inference step.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<ExcludeTool>(...)`.
/// The loop runner consumes all matching actions and removes matching tool IDs
/// from the tool descriptors before building the `InferenceRequest`.
pub struct ExcludeTool;

impl crate::model::ScheduledActionSpec for ExcludeTool {
    const KEY: &'static str = "runtime.exclude_tool";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = String;
}

/// Action spec for restricting tools to an explicit allow-list for the current inference step.
///
/// Scheduled by `BeforeInference` hooks via `cmd.schedule_action::<IncludeOnlyTools>(...)`.
/// The loop runner consumes all matching actions and intersects the tool descriptors
/// with the union of all provided tool ID lists. If any `IncludeOnlyTools` action exists,
/// only tools whose IDs appear in the combined list are kept.
pub struct IncludeOnlyTools;

impl crate::model::ScheduledActionSpec for IncludeOnlyTools {
    const KEY: &'static str = "runtime.include_only_tools";
    const PHASE: crate::model::Phase = crate::model::Phase::BeforeInference;
    type Payload = Vec<String>;
}
