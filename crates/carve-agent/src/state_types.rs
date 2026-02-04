//! Common state types for the plugin system.
//!
//! These types are used across multiple plugins and extension traits.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Tool permission behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionBehavior {
    /// Tool is allowed without confirmation.
    Allow,
    /// Tool requires user confirmation before execution.
    #[default]
    Ask,
    /// Tool is denied (will not execute).
    Deny,
}

/// Interaction request for user confirmation or input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interaction {
    /// Unique interaction ID.
    pub id: String,
    /// Type of interaction (e.g., "confirm", "input", "select").
    #[serde(rename = "type")]
    pub interaction_type: InteractionType,
    /// Message to display to the user.
    pub message: String,
    /// Optional choices for select-type interactions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<InteractionChoice>,
    /// Additional metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, Value>,
}

/// Type of user interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionType {
    /// Yes/No confirmation.
    Confirm,
    /// Free-form text input.
    Input,
    /// Selection from choices.
    Select,
}

/// A choice for select-type interactions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionChoice {
    /// Choice value (returned on selection).
    pub value: String,
    /// Human-readable label.
    pub label: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Response to an interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResponse {
    /// The interaction ID this response is for.
    pub interaction_id: String,
    /// The response value.
    pub value: InteractionValue,
}

/// Value of an interaction response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InteractionValue {
    /// Boolean response (for confirm).
    Bool(bool),
    /// String response (for input/select).
    String(String),
}

impl Interaction {
    /// Create a confirmation interaction.
    pub fn confirm(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            interaction_type: InteractionType::Confirm,
            message: message.into(),
            choices: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Create an input interaction.
    pub fn input(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            interaction_type: InteractionType::Input,
            message: message.into(),
            choices: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Create a select interaction.
    pub fn select(
        id: impl Into<String>,
        message: impl Into<String>,
        choices: Vec<InteractionChoice>,
    ) -> Self {
        Self {
            id: id.into(),
            interaction_type: InteractionType::Select,
            message: message.into(),
            choices,
            metadata: HashMap::new(),
        }
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

impl InteractionChoice {
    /// Create a new choice.
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: None,
        }
    }

    /// Add a description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_permission_behavior_default() {
        let behavior: ToolPermissionBehavior = Default::default();
        assert_eq!(behavior, ToolPermissionBehavior::Ask);
    }

    #[test]
    fn test_tool_permission_behavior_serialization() {
        let allow = ToolPermissionBehavior::Allow;
        let json = serde_json::to_string(&allow).unwrap();
        assert_eq!(json, "\"allow\"");

        let parsed: ToolPermissionBehavior = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ToolPermissionBehavior::Allow);
    }

    #[test]
    fn test_interaction_confirm() {
        let interaction = Interaction::confirm("int_1", "Are you sure?");
        assert_eq!(interaction.id, "int_1");
        assert_eq!(interaction.interaction_type, InteractionType::Confirm);
        assert_eq!(interaction.message, "Are you sure?");
        assert!(interaction.choices.is_empty());
    }

    #[test]
    fn test_interaction_input() {
        let interaction = Interaction::input("int_2", "Enter your name:");
        assert_eq!(interaction.interaction_type, InteractionType::Input);
    }

    #[test]
    fn test_interaction_select() {
        let choices = vec![
            InteractionChoice::new("yes", "Yes"),
            InteractionChoice::new("no", "No").with_description("Cancel the operation"),
        ];
        let interaction = Interaction::select("int_3", "Choose an option:", choices);

        assert_eq!(interaction.interaction_type, InteractionType::Select);
        assert_eq!(interaction.choices.len(), 2);
        assert_eq!(interaction.choices[1].description, Some("Cancel the operation".to_string()));
    }

    #[test]
    fn test_interaction_with_metadata() {
        let interaction = Interaction::confirm("int_4", "Confirm?")
            .with_metadata("tool_id", serde_json::json!("write_file"));

        assert!(interaction.metadata.contains_key("tool_id"));
    }

    #[test]
    fn test_interaction_serialization() {
        let interaction = Interaction::confirm("int_5", "Test?");
        let json = serde_json::to_string(&interaction).unwrap();
        let parsed: Interaction = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, interaction.id);
        assert_eq!(parsed.interaction_type, interaction.interaction_type);
    }

    #[test]
    fn test_interaction_response() {
        let response = InteractionResponse {
            interaction_id: "int_1".to_string(),
            value: InteractionValue::Bool(true),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: InteractionResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.interaction_id, "int_1");
        assert!(matches!(parsed.value, InteractionValue::Bool(true)));
    }
}
