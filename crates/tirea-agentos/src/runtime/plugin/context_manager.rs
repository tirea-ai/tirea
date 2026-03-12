//! Context manager: logical compression of conversation history.
//!
//! Maintains [`ContextManagerState`] with compact boundaries and artifact
//! references. Registers a [`ContextAssemblyTransform`] that replaces
//! pre-boundary messages with a summary and swaps large artifact content
//! with lightweight placeholders — without modifying the persisted thread.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tirea_contract::runtime::behavior::ReadOnlyContext;
use tirea_contract::runtime::inference::{InferenceRequestTransform, InferenceTransformOutput};
use tirea_contract::runtime::phase::{ActionSet, BeforeInferenceAction};
use tirea_contract::runtime::tool_call::ToolDescriptor;
use tirea_contract::thread::{Message, Role};
use tirea_state::State;

/// Behavior ID used for registration.
pub const CONTEXT_MANAGER_PLUGIN_ID: &str = "context_manager";

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

/// A compact boundary marking that all messages up through a given message ID
/// have been logically replaced by a summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactBoundary {
    /// UUID v7 of the last message covered by this boundary.
    pub covers_through_message_id: String,
    /// Pre-computed summary text that replaces the covered messages.
    pub summary: String,
    /// Estimated token count of the original messages that were summarized.
    pub original_token_count: usize,
    /// Timestamp (ms since epoch) when this boundary was created.
    pub created_at_ms: u64,
}

/// A reference to a large artifact whose content is replaced with a
/// lightweight placeholder during inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// Message ID containing the artifact.
    pub message_id: String,
    /// Tool call ID if the artifact is a tool result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Human-readable label for the placeholder.
    pub label: String,
    /// Original content size in characters.
    pub original_size: usize,
}

/// Thread-scoped state persisting compact boundaries and artifact references.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(
    path = "__context",
    action = "ContextManagerAction",
    scope = "thread"
)]
pub struct ContextManagerState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundaries: Vec<CompactBoundary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_refs: Vec<ArtifactRef>,
}

/// Actions that modify [`ContextManagerState`].
#[derive(Serialize, Deserialize)]
pub enum ContextManagerAction {
    /// Add a new compact boundary (later boundaries supersede earlier ones).
    AddBoundary(CompactBoundary),
    /// Register an artifact reference for placeholder substitution.
    AddArtifact(ArtifactRef),
    /// Remove artifact refs whose `message_id` is lexicographically before
    /// the given id (UUID v7 is time-ordered, so this prunes old refs).
    PruneArtifacts { before_message_id: String },
}

impl ContextManagerState {
    fn reduce(&mut self, action: ContextManagerAction) {
        match action {
            ContextManagerAction::AddBoundary(boundary) => {
                self.boundaries.push(boundary);
            }
            ContextManagerAction::AddArtifact(artifact) => {
                self.artifact_refs.push(artifact);
            }
            ContextManagerAction::PruneArtifacts { before_message_id } => {
                self.artifact_refs
                    .retain(|a| a.message_id >= before_message_id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Transform
// ---------------------------------------------------------------------------

/// Inference request transform that applies logical compression.
///
/// Constructed each step in `before_inference` with a snapshot of the current
/// [`ContextManagerState`]. The transform:
/// 1. Replaces non-system messages before the latest boundary with a summary.
/// 2. Substitutes large artifact content with lightweight placeholders.
struct ContextAssemblyTransform {
    state: ContextManagerState,
}

impl InferenceRequestTransform for ContextAssemblyTransform {
    fn transform(
        &self,
        messages: Vec<Message>,
        _tool_descriptors: &[ToolDescriptor],
    ) -> InferenceTransformOutput {
        let mut result = self.apply_boundaries(messages);
        self.apply_artifact_refs(&mut result);
        InferenceTransformOutput {
            messages: result,
            enable_prompt_cache: false,
        }
    }
}

impl ContextAssemblyTransform {
    /// Replace pre-boundary non-system messages with a summary message.
    fn apply_boundaries(&self, messages: Vec<Message>) -> Vec<Message> {
        let Some(boundary) = self.state.boundaries.last() else {
            return messages;
        };

        // Find the index of the boundary message in the input.
        let boundary_idx = messages.iter().position(|m| {
            m.id.as_deref() == Some(&boundary.covers_through_message_id)
        });

        let Some(boundary_idx) = boundary_idx else {
            // Boundary message not found (may have been filtered upstream).
            // Graceful degradation: pass through unchanged.
            return messages;
        };

        let mut out = Vec::with_capacity(messages.len());

        // Preserve system messages that appear before the boundary.
        for msg in &messages[..=boundary_idx] {
            if msg.role == Role::System {
                out.push(msg.clone());
            }
        }

        // Inject summary as an internal system message.
        out.push(Message::internal_system(format!(
            "[Summary of earlier conversation]\n{}",
            boundary.summary
        )));

        // Keep all messages after the boundary unchanged.
        for msg in &messages[boundary_idx + 1..] {
            out.push(msg.clone());
        }

        out
    }

    /// Replace content of messages referenced by artifact refs.
    fn apply_artifact_refs(&self, messages: &mut [Message]) {
        if self.state.artifact_refs.is_empty() {
            return;
        }

        for msg in messages.iter_mut() {
            let msg_id = match msg.id.as_deref() {
                Some(id) => id,
                None => continue,
            };

            for artifact in &self.state.artifact_refs {
                if artifact.message_id == msg_id {
                    msg.content = format!(
                        "[Artifact: {}] (original: {} chars)",
                        artifact.label, artifact.original_size
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Behavior that registers context assembly transforms when boundaries exist.
///
/// P0 skeleton: reads persisted state and registers the transform. Future
/// phases (P1+) will add auto-compaction triggering in `after_inference`.
#[derive(Debug, Clone, Default)]
pub struct ContextManagerPlugin;

impl ContextManagerPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl tirea_contract::runtime::AgentBehavior for ContextManagerPlugin {
    fn id(&self) -> &str {
        CONTEXT_MANAGER_PLUGIN_ID
    }

    tirea_contract::declare_plugin_states!(ContextManagerState);

    async fn before_inference(
        &self,
        ctx: &ReadOnlyContext<'_>,
    ) -> ActionSet<BeforeInferenceAction> {
        let state = ctx
            .snapshot_of::<ContextManagerState>()
            .ok()
            .unwrap_or_default();

        if state.boundaries.is_empty() && state.artifact_refs.is_empty() {
            return ActionSet::empty();
        }

        ActionSet::single(BeforeInferenceAction::AddRequestTransform(Arc::new(
            ContextAssemblyTransform { state },
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::runtime::phase::Phase;
    use tirea_contract::RunPolicy;
    use tirea_state::DocCell;

    fn make_msg_with_id(role: Role, content: &str, id: &str) -> Message {
        let mut msg = match role {
            Role::System => Message::system(content),
            Role::User => Message::user(content),
            Role::Assistant => Message::assistant(content),
            Role::Tool => Message::tool("call_0", content),
        };
        msg.id = Some(id.to_string());
        msg
    }

    // -- State tests --

    #[test]
    fn state_default() {
        let state = ContextManagerState::default();
        assert!(state.boundaries.is_empty());
        assert!(state.artifact_refs.is_empty());
    }

    #[test]
    fn reducer_add_boundary() {
        let mut state = ContextManagerState::default();
        state.reduce(ContextManagerAction::AddBoundary(CompactBoundary {
            covers_through_message_id: "msg-1".into(),
            summary: "First summary".into(),
            original_token_count: 100,
            created_at_ms: 1000,
        }));
        assert_eq!(state.boundaries.len(), 1);

        state.reduce(ContextManagerAction::AddBoundary(CompactBoundary {
            covers_through_message_id: "msg-5".into(),
            summary: "Second summary".into(),
            original_token_count: 200,
            created_at_ms: 2000,
        }));
        assert_eq!(state.boundaries.len(), 2);
        assert_eq!(state.boundaries[1].summary, "Second summary");
    }

    #[test]
    fn reducer_add_artifact() {
        let mut state = ContextManagerState::default();
        state.reduce(ContextManagerAction::AddArtifact(ArtifactRef {
            message_id: "msg-3".into(),
            tool_call_id: Some("call-1".into()),
            label: "file contents".into(),
            original_size: 50000,
        }));
        assert_eq!(state.artifact_refs.len(), 1);
        assert_eq!(state.artifact_refs[0].label, "file contents");
    }

    #[test]
    fn reducer_prune_artifacts() {
        let mut state = ContextManagerState {
            boundaries: vec![],
            artifact_refs: vec![
                ArtifactRef {
                    message_id: "aaa".into(),
                    tool_call_id: None,
                    label: "old".into(),
                    original_size: 100,
                },
                ArtifactRef {
                    message_id: "mmm".into(),
                    tool_call_id: None,
                    label: "mid".into(),
                    original_size: 200,
                },
                ArtifactRef {
                    message_id: "zzz".into(),
                    tool_call_id: None,
                    label: "new".into(),
                    original_size: 300,
                },
            ],
        };

        state.reduce(ContextManagerAction::PruneArtifacts {
            before_message_id: "mmm".into(),
        });
        assert_eq!(state.artifact_refs.len(), 2);
        assert_eq!(state.artifact_refs[0].label, "mid");
        assert_eq!(state.artifact_refs[1].label, "new");
    }

    // -- Transform tests --

    #[test]
    fn transform_no_boundaries_passthrough() {
        let transform = ContextAssemblyTransform {
            state: ContextManagerState::default(),
        };
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        let output = transform.transform(messages.clone(), &[]);
        assert_eq!(output.messages.len(), 3);
        assert_eq!(output.messages[1].content, "hello");
    }

    #[test]
    fn transform_replaces_pre_boundary_messages() {
        let state = ContextManagerState {
            boundaries: vec![CompactBoundary {
                covers_through_message_id: "msg-2".into(),
                summary: "User asked about weather, assistant replied sunny.".into(),
                original_token_count: 50,
                created_at_ms: 1000,
            }],
            artifact_refs: vec![],
        };
        let transform = ContextAssemblyTransform { state };

        let messages = vec![
            make_msg_with_id(Role::System, "You are helpful.", "sys-1"),
            make_msg_with_id(Role::User, "What is the weather?", "msg-1"),
            make_msg_with_id(Role::Assistant, "It is sunny today.", "msg-2"),
            make_msg_with_id(Role::User, "Thanks!", "msg-3"),
            make_msg_with_id(Role::Assistant, "You're welcome!", "msg-4"),
        ];

        let output = transform.transform(messages, &[]);
        // system + summary + msg-3 + msg-4 = 4
        assert_eq!(output.messages.len(), 4);
        assert_eq!(output.messages[0].role, Role::System);
        assert_eq!(output.messages[0].content, "You are helpful.");
        assert!(output.messages[1].content.contains("[Summary of earlier conversation]"));
        assert!(output.messages[1].content.contains("sunny"));
        assert_eq!(output.messages[2].content, "Thanks!");
        assert_eq!(output.messages[3].content, "You're welcome!");
    }

    #[test]
    fn transform_preserves_system_messages() {
        let state = ContextManagerState {
            boundaries: vec![CompactBoundary {
                covers_through_message_id: "msg-2".into(),
                summary: "Summary".into(),
                original_token_count: 30,
                created_at_ms: 1000,
            }],
            artifact_refs: vec![],
        };
        let transform = ContextAssemblyTransform { state };

        let messages = vec![
            make_msg_with_id(Role::System, "System prompt 1", "sys-1"),
            make_msg_with_id(Role::System, "System prompt 2", "sys-2"),
            make_msg_with_id(Role::User, "Hello", "msg-1"),
            make_msg_with_id(Role::Assistant, "Hi", "msg-2"),
            make_msg_with_id(Role::User, "Next", "msg-3"),
        ];

        let output = transform.transform(messages, &[]);
        // sys-1 + sys-2 + summary + msg-3 = 4
        assert_eq!(output.messages.len(), 4);
        assert_eq!(output.messages[0].content, "System prompt 1");
        assert_eq!(output.messages[1].content, "System prompt 2");
        assert!(output.messages[2].content.contains("[Summary"));
    }

    #[test]
    fn transform_boundary_message_not_found_passthrough() {
        let state = ContextManagerState {
            boundaries: vec![CompactBoundary {
                covers_through_message_id: "nonexistent".into(),
                summary: "Should not appear".into(),
                original_token_count: 0,
                created_at_ms: 1000,
            }],
            artifact_refs: vec![],
        };
        let transform = ContextAssemblyTransform { state };

        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        let output = transform.transform(messages.clone(), &[]);
        assert_eq!(output.messages.len(), 3);
        assert_eq!(output.messages[1].content, "hello");
    }

    #[test]
    fn transform_artifact_ref_replaces_content() {
        let state = ContextManagerState {
            boundaries: vec![],
            artifact_refs: vec![ArtifactRef {
                message_id: "tool-msg-1".into(),
                tool_call_id: Some("call-1".into()),
                label: "file.rs".into(),
                original_size: 25000,
            }],
        };
        let transform = ContextAssemblyTransform { state };

        let messages = vec![
            Message::system("sys"),
            make_msg_with_id(Role::User, "Read the file", "msg-1"),
            make_msg_with_id(
                Role::Tool,
                "fn main() { /* very long content */ }",
                "tool-msg-1",
            ),
            make_msg_with_id(Role::Assistant, "Here is the file content.", "msg-2"),
        ];

        let output = transform.transform(messages, &[]);
        assert_eq!(output.messages.len(), 4);
        assert!(output.messages[2].content.contains("[Artifact: file.rs]"));
        assert!(output.messages[2].content.contains("25000 chars"));
    }

    #[test]
    fn transform_boundary_and_artifacts_combined() {
        let state = ContextManagerState {
            boundaries: vec![CompactBoundary {
                covers_through_message_id: "msg-2".into(),
                summary: "Earlier conversation summary.".into(),
                original_token_count: 100,
                created_at_ms: 1000,
            }],
            artifact_refs: vec![ArtifactRef {
                message_id: "tool-msg-3".into(),
                tool_call_id: None,
                label: "large output".into(),
                original_size: 10000,
            }],
        };
        let transform = ContextAssemblyTransform { state };

        let messages = vec![
            make_msg_with_id(Role::System, "sys", "sys-1"),
            make_msg_with_id(Role::User, "old question", "msg-1"),
            make_msg_with_id(Role::Assistant, "old answer", "msg-2"),
            make_msg_with_id(Role::User, "new question", "msg-3-user"),
            make_msg_with_id(Role::Tool, "huge tool output here", "tool-msg-3"),
            make_msg_with_id(Role::Assistant, "final answer", "msg-4"),
        ];

        let output = transform.transform(messages, &[]);
        // sys + summary + msg-3-user + tool-msg-3(replaced) + msg-4 = 5
        assert_eq!(output.messages.len(), 5);
        assert_eq!(output.messages[0].content, "sys");
        assert!(output.messages[1].content.contains("[Summary"));
        assert_eq!(output.messages[2].content, "new question");
        assert!(output.messages[3].content.contains("[Artifact: large output]"));
        assert_eq!(output.messages[4].content, "final answer");
    }

    // -- Plugin tests --

    #[test]
    fn plugin_id() {
        let plugin = ContextManagerPlugin::new();
        use tirea_contract::runtime::AgentBehavior;
        assert_eq!(plugin.id(), "context_manager");
    }

    #[tokio::test]
    async fn plugin_before_inference_no_state() {
        let plugin = ContextManagerPlugin::new();
        use tirea_contract::runtime::AgentBehavior;

        let config = RunPolicy::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);

        let actions = plugin.before_inference(&ctx).await;
        assert!(actions.is_empty(), "no boundaries = no transform");
    }

    #[tokio::test]
    async fn plugin_before_inference_with_boundary() {
        let plugin = ContextManagerPlugin::new();
        use tirea_contract::runtime::AgentBehavior;

        let state = ContextManagerState {
            boundaries: vec![CompactBoundary {
                covers_through_message_id: "msg-5".into(),
                summary: "A summary".into(),
                original_token_count: 200,
                created_at_ms: 1000,
            }],
            artifact_refs: vec![],
        };
        let state_value = serde_json::to_value(&state).unwrap();
        let doc = DocCell::new(json!({ "__context": state_value }));
        let config = RunPolicy::new();
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);

        let actions = plugin.before_inference(&ctx).await;
        assert_eq!(actions.len(), 1, "boundary present = transform registered");

        let action = actions.into_vec().pop().unwrap();
        assert!(
            matches!(action, BeforeInferenceAction::AddRequestTransform(_)),
            "should be AddRequestTransform"
        );
    }

    #[tokio::test]
    async fn plugin_before_inference_with_artifact_only() {
        let plugin = ContextManagerPlugin::new();
        use tirea_contract::runtime::AgentBehavior;

        let state = ContextManagerState {
            boundaries: vec![],
            artifact_refs: vec![ArtifactRef {
                message_id: "msg-1".into(),
                tool_call_id: None,
                label: "test".into(),
                original_size: 5000,
            }],
        };
        let state_value = serde_json::to_value(&state).unwrap();
        let doc = DocCell::new(json!({ "__context": state_value }));
        let config = RunPolicy::new();
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);

        let actions = plugin.before_inference(&ctx).await;
        assert_eq!(actions.len(), 1, "artifact refs present = transform registered");
    }

    #[test]
    fn latest_boundary_wins() {
        let state = ContextManagerState {
            boundaries: vec![
                CompactBoundary {
                    covers_through_message_id: "msg-2".into(),
                    summary: "Old summary".into(),
                    original_token_count: 50,
                    created_at_ms: 1000,
                },
                CompactBoundary {
                    covers_through_message_id: "msg-4".into(),
                    summary: "New summary".into(),
                    original_token_count: 100,
                    created_at_ms: 2000,
                },
            ],
            artifact_refs: vec![],
        };
        let transform = ContextAssemblyTransform { state };

        let messages = vec![
            make_msg_with_id(Role::System, "sys", "sys-1"),
            make_msg_with_id(Role::User, "q1", "msg-1"),
            make_msg_with_id(Role::Assistant, "a1", "msg-2"),
            make_msg_with_id(Role::User, "q2", "msg-3"),
            make_msg_with_id(Role::Assistant, "a2", "msg-4"),
            make_msg_with_id(Role::User, "q3", "msg-5"),
        ];

        let output = transform.transform(messages, &[]);
        // sys + summary + msg-5 = 3
        assert_eq!(output.messages.len(), 3);
        assert!(output.messages[1].content.contains("New summary"));
        assert_eq!(output.messages[2].content, "q3");
    }
}
