//! Reminder prompt-segment helpers.
//!
//! Reminder persistence is backed directly by the core prompt-segment state
//! model. This crate only exposes convenience state actions plus the
//! `SystemReminder` trait used by integrations that compute reminder text.

mod actions;
mod system_reminder;

pub use actions::{add_persistent_reminder_action, add_reminder_action, clear_reminder_action};
pub use system_reminder::SystemReminder;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::runtime::state::{reduce_state_actions, ScopeContext};
    use tirea_state::apply_patch;

    #[test]
    fn add_reminder_item_writes_ephemeral_prompt_segment_state() {
        let sa = add_reminder_action("check logs");
        let patch = reduce_state_actions(vec![sa], &json!({}), "test", &ScopeContext::run())
            .expect("state action should reduce");
        let next = apply_patch(&json!({}), patch[0].patch()).expect("patch should apply");

        let items = next["__prompt_segments"]["items"]
            .as_array()
            .expect("prompt segments array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["namespace"], "reminder");
        assert_eq!(items[0]["consume"], "after_emit");
        assert_eq!(items[0]["content"], "Reminder: check logs");
    }

    #[test]
    fn add_persistent_reminder_item_writes_persistent_prompt_segment_state() {
        let sa = add_persistent_reminder_action("stay focused");
        let patch = reduce_state_actions(vec![sa], &json!({}), "test", &ScopeContext::run())
            .expect("state action should reduce");
        let next = apply_patch(&json!({}), patch[0].patch()).expect("patch should apply");

        let items = next["__prompt_segments"]["items"]
            .as_array()
            .expect("prompt segments array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["consume"], "persistent");
    }

    #[test]
    fn clear_reminder_action_clears_prompt_segment_namespace() {
        let base = json!({
            "__prompt_segments": {
                "items": [
                    {
                        "namespace": "reminder",
                        "key": "a",
                        "content": "Reminder: check logs",
                        "cooldown_turns": 0,
                        "target": "session",
                        "consume": "after_emit"
                    },
                    {
                        "namespace": "skill",
                        "key": "b",
                        "content": "tail",
                        "cooldown_turns": 0,
                        "target": "suffix_system",
                        "consume": "persistent"
                    }
                ]
            }
        });
        let patch = reduce_state_actions(
            vec![clear_reminder_action()],
            &base,
            "test",
            &ScopeContext::run(),
        )
        .expect("state action should reduce");
        let next = apply_patch(&base, patch[0].patch()).expect("patch should apply");

        let items = next["__prompt_segments"]["items"]
            .as_array()
            .expect("prompt segments array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["namespace"], "skill");
    }

    #[test]
    fn system_reminder_trait_can_be_implemented() {
        struct StaticReminder;

        #[async_trait::async_trait]
        impl SystemReminder for StaticReminder {
            fn id(&self) -> &str {
                "static"
            }

            async fn remind(
                &self,
                _ctx: &tirea_contract::runtime::tool_call::ToolCallContext<'_>,
            ) -> Option<String> {
                Some("remember".into())
            }
        }

        let _ = StaticReminder;
    }
}
