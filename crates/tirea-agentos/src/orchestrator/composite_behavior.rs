use crate::contracts::runtime::plugin::agent::{AgentBehavior, ReadOnlyContext};
use crate::contracts::runtime::plugin::phase::action::Action;
use async_trait::async_trait;
use std::sync::Arc;

/// Compose multiple behaviors into a single [`AgentBehavior`].
///
/// If the list contains a single behavior, returns it directly.
/// If it contains multiple, wraps them in a composite that concatenates
/// their action lists in order.
///
/// This is the public API for behavior composition — callers never need
/// to know about the concrete composite type.
pub fn compose_behaviors(
    id: impl Into<String>,
    behaviors: Vec<Arc<dyn AgentBehavior>>,
) -> Arc<dyn AgentBehavior> {
    match behaviors.len() {
        0 => Arc::new(crate::contracts::runtime::plugin::agent::NoOpBehavior),
        1 => behaviors.into_iter().next().unwrap(),
        _ => Arc::new(CompositeBehavior::new(id, behaviors)),
    }
}

/// An [`AgentBehavior`] that composes multiple sub-behaviors.
///
/// Each phase hook iterates all sub-behaviors in order, concatenating their
/// action lists into a single result. All sub-behaviors receive the same
/// [`ReadOnlyContext`] snapshot — they do not see each other's effects
/// within the same phase. The loop validates and applies all collected
/// actions sequentially after the composite hook returns.
pub(crate) struct CompositeBehavior {
    id: String,
    behaviors: Vec<Arc<dyn AgentBehavior>>,
}

impl CompositeBehavior {
    pub(crate) fn new(id: impl Into<String>, behaviors: Vec<Arc<dyn AgentBehavior>>) -> Self {
        Self {
            id: id.into(),
            behaviors,
        }
    }
}

#[async_trait]
impl AgentBehavior for CompositeBehavior {
    fn id(&self) -> &str {
        &self.id
    }

    fn behavior_ids(&self) -> Vec<&str> {
        self.behaviors
            .iter()
            .flat_map(|b| b.behavior_ids())
            .collect()
    }

    async fn run_start(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.run_start(ctx).await);
        }
        merged
    }

    async fn step_start(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.step_start(ctx).await);
        }
        merged
    }

    async fn before_inference(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.before_inference(ctx).await);
        }
        merged
    }

    async fn after_inference(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.after_inference(ctx).await);
        }
        merged
    }

    async fn before_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.before_tool_execute(ctx).await);
        }
        merged
    }

    async fn after_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.after_tool_execute(ctx).await);
        }
        merged
    }

    async fn step_end(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.step_end(ctx).await);
        }
        merged
    }

    async fn run_end(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.run_end(ctx).await);
        }
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::runtime::plugin::phase::core::actions::{AddSystemContext, BlockTool};
    use crate::contracts::runtime::plugin::phase::Phase;
    use crate::contracts::RunConfig;
    use serde_json::json;
    use tirea_state::DocCell;

    struct ContextBehavior {
        id: String,
        text: String,
    }

    #[async_trait]
    impl AgentBehavior for ContextBehavior {
        fn id(&self) -> &str {
            &self.id
        }

        async fn before_inference(&self, _ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
            vec![Box::new(AddSystemContext(self.text.clone()))]
        }
    }

    struct BlockBehavior;

    #[async_trait]
    impl AgentBehavior for BlockBehavior {
        fn id(&self) -> &str {
            "blocker"
        }

        async fn before_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
            if ctx.tool_name() == Some("dangerous") {
                vec![Box::new(BlockTool("denied".into()))]
            } else {
                vec![]
            }
        }
    }

    fn make_ctx<'a>(
        doc: &'a DocCell,
        run_config: &'a RunConfig,
        phase: Phase,
    ) -> ReadOnlyContext<'a> {
        ReadOnlyContext::new(phase, "thread_1", &[], run_config, doc)
    }

    #[tokio::test]
    async fn composite_merges_actions() {
        let behaviors: Vec<Arc<dyn AgentBehavior>> = vec![
            Arc::new(ContextBehavior {
                id: "a".into(),
                text: "ctx_a".into(),
            }),
            Arc::new(ContextBehavior {
                id: "b".into(),
                text: "ctx_b".into(),
            }),
        ];
        let composite = CompositeBehavior::new("test", behaviors);

        let doc = DocCell::new(json!({}));
        let run_config = RunConfig::new();
        let ctx = make_ctx(&doc, &run_config, Phase::BeforeInference);
        let actions = composite.before_inference(&ctx).await;

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].label(), "add_system_context");
        assert_eq!(actions[1].label(), "add_system_context");
    }

    #[tokio::test]
    async fn composite_empty_behaviors_returns_empty() {
        let composite = CompositeBehavior::new("empty", vec![]);
        let doc = DocCell::new(json!({}));
        let run_config = RunConfig::new();
        let ctx = make_ctx(&doc, &run_config, Phase::BeforeInference);

        let actions = composite.before_inference(&ctx).await;
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn composite_preserves_action_order() {
        let behaviors: Vec<Arc<dyn AgentBehavior>> = vec![
            Arc::new(ContextBehavior {
                id: "first".into(),
                text: "1".into(),
            }),
            Arc::new(BlockBehavior),
            Arc::new(ContextBehavior {
                id: "last".into(),
                text: "2".into(),
            }),
        ];
        let composite = CompositeBehavior::new("order_test", behaviors);

        let doc = DocCell::new(json!({}));
        let run_config = RunConfig::new();
        let ctx = make_ctx(&doc, &run_config, Phase::BeforeInference);
        let actions = composite.before_inference(&ctx).await;

        // BlockBehavior returns empty for BeforeInference, so 2 actions
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].label(), "add_system_context");
        assert_eq!(actions[1].label(), "add_system_context");
    }

    #[test]
    fn compose_behaviors_empty_returns_noop() {
        let behavior = compose_behaviors("test", Vec::new());

        assert_eq!(behavior.id(), "noop");
        assert_eq!(behavior.behavior_ids(), vec!["noop"]);
    }

    #[test]
    fn compose_behaviors_single_passthrough() {
        let input = Arc::new(ContextBehavior {
            id: "single".into(),
            text: "ctx".into(),
        }) as Arc<dyn AgentBehavior>;
        let behavior = compose_behaviors("ignored", vec![input.clone()]);

        assert!(Arc::ptr_eq(&behavior, &input));
        assert_eq!(behavior.id(), "single");
        assert_eq!(behavior.behavior_ids(), vec!["single"]);
    }

    #[test]
    fn compose_behaviors_multiple_keeps_leaf_behavior_ids_order() {
        let behavior = compose_behaviors(
            "composed",
            vec![
                Arc::new(ContextBehavior {
                    id: "a".into(),
                    text: "ctx_a".into(),
                }),
                Arc::new(ContextBehavior {
                    id: "b".into(),
                    text: "ctx_b".into(),
                }),
            ],
        );

        assert_eq!(behavior.id(), "composed");
        assert_eq!(behavior.behavior_ids(), vec!["a", "b"]);
    }
}
