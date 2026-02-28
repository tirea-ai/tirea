use crate::contracts::runtime::plugin::agent::{AgentBehavior, ReadOnlyContext};
use crate::contracts::runtime::plugin::phase::effect::PhaseOutput;
use crate::contracts::runtime::plugin::phase::{AnyPluginAction, Phase};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tirea_state::TrackedPatch;

/// Compose multiple behaviors into a single [`AgentBehavior`].
///
/// If the list contains a single behavior, returns it directly.
/// If it contains multiple, wraps them in a composite that merges
/// their [`PhaseOutput`]s in order.
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
/// Each phase hook iterates all sub-behaviors in order, collecting their
/// [`PhaseOutput`]s into a single merged result. All sub-behaviors receive
/// the same [`ReadOnlyContext`] snapshot — they do not see each other's
/// effects within the same phase. The loop applies all collected effects
/// sequentially after the composite hook returns.
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

/// Merge `source` effects and state actions into `target`.
fn merge_output(target: &mut PhaseOutput, source: PhaseOutput) {
    target.effects.extend(source.effects);
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

    async fn run_start(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.run_start(ctx).await);
        }
        merged
    }

    async fn step_start(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.step_start(ctx).await);
        }
        merged
    }

    async fn before_inference(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.before_inference(ctx).await);
        }
        merged
    }

    async fn after_inference(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.after_inference(ctx).await);
        }
        merged
    }

    async fn before_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.before_tool_execute(ctx).await);
        }
        merged
    }

    async fn after_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.after_tool_execute(ctx).await);
        }
        merged
    }

    async fn step_end(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.step_end(ctx).await);
        }
        merged
    }

    async fn run_end(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
        let mut merged = PhaseOutput::default();
        for b in &self.behaviors {
            merge_output(&mut merged, b.run_end(ctx).await);
        }
        merged
    }

    async fn phase_actions(&self, phase: Phase, ctx: &ReadOnlyContext<'_>) -> Vec<AnyPluginAction> {
        let mut merged = Vec::new();
        for b in &self.behaviors {
            merged.extend(b.phase_actions(phase, ctx).await);
        }
        merged
    }

    fn reduce_plugin_actions(
        &self,
        actions: Vec<AnyPluginAction>,
        base_snapshot: &Value,
    ) -> Result<Vec<TrackedPatch>, String> {
        if actions.is_empty() {
            return Ok(Vec::new());
        }

        let mut grouped: HashMap<String, Vec<AnyPluginAction>> = HashMap::new();
        for action in actions {
            grouped
                .entry(action.plugin_id().to_string())
                .or_default()
                .push(action);
        }

        let mut merged_patches = Vec::new();
        for behavior in &self.behaviors {
            let mut behavior_actions = Vec::new();
            for id in behavior.behavior_ids() {
                if let Some(mut actions) = grouped.remove(id) {
                    behavior_actions.append(&mut actions);
                }
            }
            if behavior_actions.is_empty() {
                continue;
            }

            let mut patches = behavior.reduce_plugin_actions(behavior_actions, base_snapshot)?;
            merged_patches.append(&mut patches);
        }

        if !grouped.is_empty() {
            let mut ids: Vec<String> = grouped.into_keys().collect();
            ids.sort();
            return Err(format!(
                "composite behavior '{}' cannot route plugin actions for ids: {:?}",
                self.id, ids
            ));
        }

        Ok(merged_patches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::runtime::plugin::phase::effect::PhaseEffect;
    use crate::contracts::runtime::plugin::phase::state_spec::{
        reduce_state_actions, AnyStateAction,
    };
    use crate::contracts::runtime::plugin::phase::Phase;
    use crate::contracts::RunConfig;
    use serde_json::json;
    use tirea_state::{path, DocCell, Op, Patch, TrackedPatch};

    struct ContextBehavior {
        id: String,
        text: String,
    }

    #[async_trait]
    impl AgentBehavior for ContextBehavior {
        fn id(&self) -> &str {
            &self.id
        }

        async fn before_inference(&self, _ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
            PhaseOutput::new().system_context(&self.text)
        }
    }

    struct BlockBehavior;

    #[async_trait]
    impl AgentBehavior for BlockBehavior {
        fn id(&self) -> &str {
            "blocker"
        }

        async fn before_tool_execute(&self, ctx: &ReadOnlyContext<'_>) -> PhaseOutput {
            if ctx.tool_name() == Some("dangerous") {
                PhaseOutput::new().block_tool("denied")
            } else {
                PhaseOutput::default()
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
    async fn composite_merges_effects() {
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
        let output = composite.before_inference(&ctx).await;

        assert_eq!(output.effects.len(), 2);
        assert!(matches!(&output.effects[0], PhaseEffect::SystemContext(s) if s == "ctx_a"));
        assert!(matches!(&output.effects[1], PhaseEffect::SystemContext(s) if s == "ctx_b"));
    }

    #[tokio::test]
    async fn composite_routes_phase_actions_to_child_reducers() {
        struct RoutedPatchBehavior {
            id: &'static str,
            key: &'static str,
        }

        #[async_trait]
        impl AgentBehavior for RoutedPatchBehavior {
            fn id(&self) -> &str {
                self.id
            }

            async fn phase_actions(
                &self,
                phase: Phase,
                _ctx: &ReadOnlyContext<'_>,
            ) -> Vec<AnyPluginAction> {
                if phase != Phase::BeforeInference {
                    return Vec::new();
                }

                vec![AnyPluginAction::new(self.id(), self.key.to_string())]
            }

            fn reduce_plugin_actions(
                &self,
                actions: Vec<AnyPluginAction>,
                base_snapshot: &Value,
            ) -> Result<Vec<TrackedPatch>, String> {
                let mut state_actions = Vec::new();
                for action in actions {
                    if action.plugin_id() != self.id {
                        return Err(format!(
                            "plugin '{}' received action for '{}'",
                            self.id,
                            action.plugin_id()
                        ));
                    }
                    let key = action.downcast::<String>().map_err(|other| {
                        format!(
                            "plugin '{}' failed to downcast action '{}'",
                            self.id,
                            other.action_type_name()
                        )
                    })?;
                    let patch = TrackedPatch::new(
                        Patch::new().with_op(Op::set(path!("debug", key), json!(true))),
                    )
                    .with_source(self.id);
                    state_actions.push(AnyStateAction::Patch(patch));
                }
                reduce_state_actions(state_actions, base_snapshot, &format!("plugin:{}", self.id))
                    .map_err(|e| e.to_string())
            }
        }

        let behaviors: Vec<Arc<dyn AgentBehavior>> = vec![
            Arc::new(RoutedPatchBehavior {
                id: "a",
                key: "from_a",
            }),
            Arc::new(RoutedPatchBehavior {
                id: "b",
                key: "from_b",
            }),
        ];
        let composite = CompositeBehavior::new("test", behaviors);

        let doc = DocCell::new(json!({}));
        let run_config = RunConfig::new();
        let ctx = make_ctx(&doc, &run_config, Phase::BeforeInference);
        let actions = composite.phase_actions(Phase::BeforeInference, &ctx).await;
        assert_eq!(actions.len(), 2);
        let patches = composite
            .reduce_plugin_actions(actions, &ctx.snapshot())
            .expect("composite should route actions");

        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].source.as_deref(), Some("a"));
        assert_eq!(patches[1].source.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn composite_empty_behaviors_returns_empty() {
        let composite = CompositeBehavior::new("empty", vec![]);
        let doc = DocCell::new(json!({}));
        let run_config = RunConfig::new();
        let ctx = make_ctx(&doc, &run_config, Phase::BeforeInference);

        let output = composite.before_inference(&ctx).await;
        assert!(output.is_empty());
    }

    #[tokio::test]
    async fn composite_preserves_effect_order() {
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
        let output = composite.before_inference(&ctx).await;

        // BlockBehavior returns empty for BeforeInference, so 2 effects
        assert_eq!(output.effects.len(), 2);
        assert!(matches!(&output.effects[0], PhaseEffect::SystemContext(s) if s == "1"));
        assert!(matches!(&output.effects[1], PhaseEffect::SystemContext(s) if s == "2"));
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
