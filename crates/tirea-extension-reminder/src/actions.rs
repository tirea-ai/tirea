use super::state::{ReminderAction, ReminderState};
use tirea_contract::runtime::phase::{ActionSet, BeforeInferenceAction};
use tirea_contract::runtime::state::AnyStateAction;

/// Create a state action that adds a reminder item.
pub fn add_reminder_action(text: impl Into<String>) -> AnyStateAction {
    AnyStateAction::new::<ReminderState>(ReminderAction::Add { text: text.into() })
}

/// Inject reminder texts as session context entries.
pub fn inject_reminders(texts: Vec<String>) -> ActionSet<BeforeInferenceAction> {
    texts
        .into_iter()
        .map(BeforeInferenceAction::AddSessionContext)
        .collect::<Vec<_>>()
        .into()
}

/// Create a state action that clears reminder state.
pub fn clear_reminder_action() -> AnyStateAction {
    AnyStateAction::new::<ReminderState>(ReminderAction::Clear)
}

/// Add a reminder item via typed state action.
pub struct AddReminderItem(pub String);

impl AddReminderItem {
    pub fn into_state_action(self) -> AnyStateAction {
        add_reminder_action(self.0)
    }
}

/// Inject reminder texts into session context.
pub struct InjectReminders(pub Vec<String>);

impl InjectReminders {
    pub fn into_action_set(self) -> ActionSet<BeforeInferenceAction> {
        inject_reminders(self.0)
    }
}

/// Clear reminder state after injection.
pub struct ClearReminderState;

impl ClearReminderState {
    pub fn into_state_action(self) -> AnyStateAction {
        clear_reminder_action()
    }
}
