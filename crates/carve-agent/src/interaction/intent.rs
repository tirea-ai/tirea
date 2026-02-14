//! Interaction intent channel shared by strategy plugins and the interaction mechanism plugin.

use crate::phase::StepContext;
use crate::state_types::Interaction;
use serde::{Deserialize, Serialize};

const INTERACTION_INTENTS_KEY: &str = "__interaction_intents";

/// Strategy-produced intent consumed by [`super::interaction_plugin::InteractionPlugin`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) enum InteractionIntent {
    /// Mark current tool as pending with a client interaction payload.
    Pending { interaction: Interaction },
}

fn read_intents(step: &StepContext<'_>) -> Vec<InteractionIntent> {
    step.scratchpad_get(INTERACTION_INTENTS_KEY)
        .unwrap_or_default()
}

fn write_intents(step: &mut StepContext<'_>, intents: Vec<InteractionIntent>) {
    let _ = step.scratchpad_set(INTERACTION_INTENTS_KEY, intents);
}

pub(crate) fn push_intent(step: &mut StepContext<'_>, intent: InteractionIntent) {
    let mut intents = read_intents(step);
    intents.push(intent);
    write_intents(step, intents);
}

pub(crate) fn push_pending_intent(step: &mut StepContext<'_>, interaction: Interaction) {
    push_intent(step, InteractionIntent::Pending { interaction });
}

pub(crate) fn take_intents(step: &mut StepContext<'_>) -> Vec<InteractionIntent> {
    let intents = read_intents(step);
    write_intents(step, Vec::new());
    intents
}
