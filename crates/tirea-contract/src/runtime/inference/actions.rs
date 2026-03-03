use crate::runtime::{
    inference::{InferenceContext, MessagingContext},
    phase::StepContext,
    Action, Phase,
};

/// Append a line to the system prompt during `BeforeInference`.
pub struct AddSystemContext(pub String);

impl Action for AddSystemContext {
    fn label(&self) -> &'static str {
        "add_system_context"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference {
            Ok(())
        } else {
            Err(format!(
                "AddSystemContext is only allowed in BeforeInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<InferenceContext>()
            .system_context
            .push(self.0);
    }
}

/// Inject a session context message during `BeforeInference`.
pub struct AddSessionContext(pub String);

impl Action for AddSessionContext {
    fn label(&self) -> &'static str {
        "add_session_context"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference {
            Ok(())
        } else {
            Err(format!(
                "AddSessionContext is only allowed in BeforeInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<InferenceContext>()
            .session_context
            .push(self.0);
    }
}

/// Add a system reminder after tool execution (`AfterToolExecute`).
///
/// Reminders are wrapped in `<system-reminder>` tags and injected into the
/// thread as internal system messages following the tool result.
pub struct AddSystemReminder(pub String);

impl Action for AddSystemReminder {
    fn label(&self) -> &'static str {
        "add_system_reminder"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::AfterToolExecute {
            Ok(())
        } else {
            Err(format!(
                "AddSystemReminder is only allowed in AfterToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<MessagingContext>()
            .reminders
            .push(self.0);
    }
}

/// Append a user message after tool execution (`AfterToolExecute`).
pub struct AddUserMessage(pub String);

impl Action for AddUserMessage {
    fn label(&self) -> &'static str {
        "add_user_message"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::AfterToolExecute {
            Ok(())
        } else {
            Err(format!(
                "AddUserMessage is only allowed in AfterToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<MessagingContext>()
            .user_messages
            .push(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_system_context_phase_validation() {
        let action = AddSystemContext("ctx".into());
        assert!(action.validate(Phase::BeforeInference).is_ok());
        assert!(action.validate(Phase::AfterToolExecute).is_err());
        assert!(action.validate(Phase::StepStart).is_err());
    }

    #[test]
    fn add_session_context_phase_validation() {
        let action = AddSessionContext("ctx".into());
        assert!(action.validate(Phase::BeforeInference).is_ok());
        assert!(action.validate(Phase::AfterToolExecute).is_err());
    }

    #[test]
    fn add_system_reminder_phase_validation() {
        let action = AddSystemReminder("remember".into());
        assert!(action.validate(Phase::AfterToolExecute).is_ok());
        assert!(action.validate(Phase::BeforeInference).is_err());
        assert!(action.validate(Phase::StepStart).is_err());
    }

    #[test]
    fn add_user_message_phase_validation() {
        let action = AddUserMessage("msg".into());
        assert!(action.validate(Phase::AfterToolExecute).is_ok());
        assert!(action.validate(Phase::BeforeInference).is_err());
    }
}
