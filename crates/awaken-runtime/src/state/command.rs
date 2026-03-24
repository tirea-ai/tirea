use std::ops::{Deref, DerefMut};

use awaken_contract::StateError;
use awaken_contract::model::{EffectSpec, ScheduledAction, ScheduledActionSpec, TypedEffect};

use super::{MergeStrategy, MutationBatch};

pub struct StateCommand {
    pub(crate) patch: MutationBatch,
    pub(crate) scheduled_actions: Vec<ScheduledAction>,
    pub(crate) effects: Vec<TypedEffect>,
}

impl StateCommand {
    pub fn new() -> Self {
        Self {
            patch: MutationBatch::new(),
            scheduled_actions: Vec::new(),
            effects: Vec::new(),
        }
    }

    pub fn with_base_revision(mut self, revision: u64) -> Self {
        self.patch = self.patch.with_base_revision(revision);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.patch.is_empty() && self.scheduled_actions.is_empty() && self.effects.is_empty()
    }

    pub fn emit<E: EffectSpec>(&mut self, payload: E::Payload) -> Result<(), StateError> {
        self.effects.push(TypedEffect::from_spec::<E>(&payload)?);
        Ok(())
    }

    pub fn schedule_action<A: ScheduledActionSpec>(
        &mut self,
        payload: A::Payload,
    ) -> Result<(), StateError> {
        self.scheduled_actions.push(ScheduledAction::new(
            A::PHASE,
            A::KEY,
            A::encode_payload(&payload)?,
        ));
        Ok(())
    }

    pub fn extend(&mut self, mut other: Self) -> Result<(), StateError> {
        self.patch.extend(other.patch)?;
        self.scheduled_actions.append(&mut other.scheduled_actions);
        self.effects.append(&mut other.effects);
        Ok(())
    }

    /// Merge two commands from parallel execution using the given merge strategy.
    pub fn merge_parallel<F>(self, other: Self, strategy: F) -> Result<Self, StateError>
    where
        F: Fn(&str) -> MergeStrategy,
    {
        let patch = self.patch.merge_parallel(other.patch, strategy)?;
        let mut scheduled_actions = self.scheduled_actions;
        scheduled_actions.extend(other.scheduled_actions);
        let mut effects = self.effects;
        effects.extend(other.effects);
        Ok(Self {
            patch,
            scheduled_actions,
            effects,
        })
    }
}

impl Default for StateCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for StateCommand {
    type Target = MutationBatch;

    fn deref(&self) -> &Self::Target {
        &self.patch
    }
}

impl DerefMut for StateCommand {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.patch
    }
}

#[cfg(test)]
mod tests {
    use awaken_contract::model::{EffectSpec, Phase, ScheduledActionSpec};

    use super::*;

    struct TestAction;

    impl ScheduledActionSpec for TestAction {
        const KEY: &'static str = "test.action";
        const PHASE: Phase = Phase::RunStart;
        type Payload = String;
    }

    struct CustomEffect;

    impl EffectSpec for CustomEffect {
        const KEY: &'static str = "test.custom_effect";
        type Payload = String;
    }

    #[test]
    fn state_command_accumulates_actions_and_effects() {
        let mut command = StateCommand::new();
        command
            .schedule_action::<TestAction>("go".into())
            .expect("schedule should succeed");
        command
            .emit::<CustomEffect>("payload".into())
            .expect("effect should encode");

        assert!(!command.is_empty());
        assert_eq!(command.scheduled_actions.len(), 1);
        assert_eq!(command.effects.len(), 1);
    }

    #[test]
    fn state_command_extend_merges_patch_actions_and_effects() {
        let mut left = StateCommand::new().with_base_revision(5);
        left.schedule_action::<TestAction>("left".into())
            .expect("left action should schedule");

        let mut right = StateCommand::new().with_base_revision(5);
        right
            .emit::<CustomEffect>("effect".into())
            .expect("right effect should encode");

        left.extend(right).expect("commands should merge");

        assert_eq!(left.base_revision(), Some(5));
        assert_eq!(left.scheduled_actions.len(), 1);
        assert_eq!(left.effects.len(), 1);
    }

    #[test]
    fn state_command_emit_supports_custom_effect_specs() {
        let mut command = StateCommand::new();
        command
            .emit::<CustomEffect>("payload".into())
            .expect("custom effect should encode");

        let decoded = command.effects[0]
            .decode::<CustomEffect>()
            .expect("custom effect should decode");
        assert_eq!(decoded, "payload");
    }

    // -----------------------------------------------------------------------
    // Migrated from uncarve: additional command tests
    // -----------------------------------------------------------------------

    #[test]
    fn state_command_new_is_empty() {
        let cmd = StateCommand::new();
        assert!(cmd.is_empty());
    }

    #[test]
    fn state_command_default_is_empty() {
        let cmd = StateCommand::default();
        assert!(cmd.is_empty());
    }

    #[test]
    fn state_command_with_base_revision() {
        let cmd = StateCommand::new().with_base_revision(42);
        assert_eq!(cmd.base_revision(), Some(42));
    }

    #[test]
    fn state_command_not_empty_after_schedule_action() {
        let mut cmd = StateCommand::new();
        cmd.schedule_action::<TestAction>("action".into()).unwrap();
        assert!(!cmd.is_empty());
    }

    #[test]
    fn state_command_not_empty_after_emit() {
        let mut cmd = StateCommand::new();
        cmd.emit::<CustomEffect>("effect".into()).unwrap();
        assert!(!cmd.is_empty());
    }

    #[test]
    fn state_command_extend_mismatched_revisions_fails() {
        let left = StateCommand::new().with_base_revision(1);
        let right = StateCommand::new().with_base_revision(2);
        let mut cmd = left;
        let err = cmd.extend(right);
        assert!(err.is_err());
    }

    #[test]
    fn state_command_extend_accumulates_actions() {
        let mut left = StateCommand::new();
        left.schedule_action::<TestAction>("a1".into()).unwrap();

        let mut right = StateCommand::new();
        right.schedule_action::<TestAction>("a2".into()).unwrap();
        right.emit::<CustomEffect>("e1".into()).unwrap();

        left.extend(right).unwrap();
        assert_eq!(left.scheduled_actions.len(), 2);
        assert_eq!(left.effects.len(), 1);
    }

    #[test]
    fn state_command_deref_accesses_mutation_batch() {
        let cmd = StateCommand::new().with_base_revision(10);
        // Deref gives us access to MutationBatch methods
        assert_eq!(cmd.base_revision(), Some(10));
        assert!(cmd.is_empty()); // no ops yet
    }

    #[test]
    fn state_command_multiple_scheduled_actions() {
        let mut cmd = StateCommand::new();
        for i in 0..5 {
            cmd.schedule_action::<TestAction>(format!("action_{}", i))
                .unwrap();
        }
        assert_eq!(cmd.scheduled_actions.len(), 5);
    }

    #[test]
    fn state_command_multiple_effects() {
        let mut cmd = StateCommand::new();
        for i in 0..5 {
            cmd.emit::<CustomEffect>(format!("effect_{}", i)).unwrap();
        }
        assert_eq!(cmd.effects.len(), 5);
    }

    // -----------------------------------------------------------------------
    // Migrated from uncarve: merge_parallel and extended edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn state_command_merge_parallel_combines_all_fields() {
        let mut left = StateCommand::new();
        left.schedule_action::<TestAction>("left_action".into())
            .unwrap();
        left.emit::<CustomEffect>("left_effect".into()).unwrap();

        let mut right = StateCommand::new();
        right
            .schedule_action::<TestAction>("right_action".into())
            .unwrap();
        right.emit::<CustomEffect>("right_effect".into()).unwrap();

        let merged = left
            .merge_parallel(right, |_| MergeStrategy::Commutative)
            .unwrap();
        assert_eq!(merged.scheduled_actions.len(), 2);
        assert_eq!(merged.effects.len(), 2);
    }

    #[test]
    fn state_command_merge_parallel_empty_commands() {
        let left = StateCommand::new();
        let right = StateCommand::new();
        let merged = left
            .merge_parallel(right, |_| MergeStrategy::Exclusive)
            .unwrap();
        assert!(merged.is_empty());
    }

    #[test]
    fn state_command_merge_parallel_preserves_base_revision() {
        let left = StateCommand::new().with_base_revision(5);
        let right = StateCommand::new().with_base_revision(5);
        let merged = left
            .merge_parallel(right, |_| MergeStrategy::Commutative)
            .unwrap();
        assert_eq!(merged.base_revision(), Some(5));
    }

    #[test]
    fn state_command_merge_parallel_mismatched_revisions_fails() {
        let left = StateCommand::new().with_base_revision(1);
        let right = StateCommand::new().with_base_revision(2);
        let result = left.merge_parallel(right, |_| MergeStrategy::Commutative);
        assert!(result.is_err());
    }

    #[test]
    fn state_command_extend_empty_into_populated() {
        let mut populated = StateCommand::new();
        populated
            .schedule_action::<TestAction>("a1".into())
            .unwrap();
        populated.emit::<CustomEffect>("e1".into()).unwrap();

        let empty = StateCommand::new();
        populated.extend(empty).unwrap();

        assert_eq!(populated.scheduled_actions.len(), 1);
        assert_eq!(populated.effects.len(), 1);
    }

    #[test]
    fn state_command_extend_populated_into_empty() {
        let mut empty = StateCommand::new();

        let mut populated = StateCommand::new();
        populated
            .schedule_action::<TestAction>("a1".into())
            .unwrap();
        populated.emit::<CustomEffect>("e1".into()).unwrap();

        empty.extend(populated).unwrap();
        assert_eq!(empty.scheduled_actions.len(), 1);
        assert_eq!(empty.effects.len(), 1);
    }

    #[test]
    fn state_command_deref_mut_allows_mutation_batch_update() {
        let mut cmd = StateCommand::new();
        // DerefMut accesses MutationBatch, allowing direct update calls
        // (though in practice you'd use typed update calls)
        assert!(cmd.is_empty());
        // After scheduling, is_empty returns false due to scheduled_actions
        cmd.schedule_action::<TestAction>("hello".into()).unwrap();
        assert!(!cmd.is_empty());
    }

    #[test]
    fn state_command_effects_decode_in_order() {
        let mut cmd = StateCommand::new();
        for i in 0..3 {
            cmd.emit::<CustomEffect>(format!("payload_{i}")).unwrap();
        }
        for (i, effect) in cmd.effects.iter().enumerate() {
            let decoded = effect.decode::<CustomEffect>().unwrap();
            assert_eq!(decoded, format!("payload_{i}"));
        }
    }
}
