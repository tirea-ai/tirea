use std::marker::PhantomData;

use crate::foundation::*;

pub(crate) trait MutationOp: Send {
    fn apply(self: Box<Self>, state: &mut Snapshot);
}

pub(crate) trait SlotMutationTarget {
    type Update: Send + 'static;

    fn apply(state: &mut Snapshot, update: Self::Update);
}

impl<K> SlotMutationTarget for K
where
    K: StateSlot,
{
    type Update = K::Update;

    fn apply(state: &mut Snapshot, update: Self::Update) {
        let value = std::sync::Arc::make_mut(&mut state.ext).get_or_insert_default::<K>();
        K::apply(value, update);
    }
}

struct SlotPatch<S: SlotMutationTarget> {
    update: Option<S::Update>,
    _marker: PhantomData<S>,
}

impl<S> SlotPatch<S>
where
    S: SlotMutationTarget,
{
    fn new(update: S::Update) -> Self {
        Self {
            update: Some(update),
            _marker: PhantomData,
        }
    }
}

impl<S> MutationOp for SlotPatch<S>
where
    S: SlotMutationTarget + Send,
{
    fn apply(mut self: Box<Self>, state: &mut Snapshot) {
        let update = self.update.take().expect("slot patch already applied");
        S::apply(state, update);
    }
}

struct ClearSlotMutation {
    clear: fn(&mut SlotMap),
}

impl ClearSlotMutation {
    fn new(clear: fn(&mut SlotMap)) -> Self {
        Self { clear }
    }
}

impl MutationOp for ClearSlotMutation {
    fn apply(self: Box<Self>, state: &mut Snapshot) {
        (self.clear)(std::sync::Arc::make_mut(&mut state.ext));
    }
}

pub struct MutationBatch {
    pub(crate) base_revision: Option<u64>,
    pub(crate) ops: Vec<Box<dyn MutationOp>>,
    pub(crate) touched_slot_keys: Vec<String>,
}

impl MutationBatch {
    pub fn new() -> Self {
        Self {
            base_revision: None,
            ops: Vec::new(),
            touched_slot_keys: Vec::new(),
        }
    }

    pub fn with_base_revision(mut self, revision: u64) -> Self {
        self.base_revision = Some(revision);
        self
    }

    pub fn base_revision(&self) -> Option<u64> {
        self.base_revision
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn update<K>(&mut self, update: K::Update)
    where
        K: StateSlot,
    {
        self.ops.push(Box::new(SlotPatch::<K>::new(update)));
        self.touched_slot_keys.push(K::KEY.to_string());
    }

    pub(crate) fn clear_extension_with(&mut self, key: impl Into<String>, clear: fn(&mut SlotMap)) {
        self.ops.push(Box::new(ClearSlotMutation::new(clear)));
        self.touched_slot_keys.push(key.into());
    }

    pub fn extend(&mut self, mut other: Self) -> Result<(), StateError> {
        self.base_revision = match (self.base_revision, other.base_revision) {
            (Some(left), Some(right)) if left != right => {
                return Err(StateError::MutationBaseRevisionMismatch { left, right });
            }
            (Some(left), _) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        };

        self.ops.append(&mut other.ops);
        self.touched_slot_keys.append(&mut other.touched_slot_keys);
        Ok(())
    }

    pub(crate) fn op_len(&self) -> usize {
        self.ops.len()
    }
}

impl Default for MutationBatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Counter;

    impl StateSlot for Counter {
        const KEY: &'static str = "counter";
        type Value = usize;
        type Update = usize;

        fn apply(value: &mut Self::Value, update: Self::Update) {
            *value += update;
        }
    }

    #[test]
    fn mutation_batch_merges_matching_base_revisions() {
        let mut left = MutationBatch::new().with_base_revision(3);
        left.update::<Counter>(1);

        let mut right = MutationBatch::new().with_base_revision(3);
        right.update::<Counter>(2);

        left.extend(right)
            .expect("matching base revisions should merge");
        assert_eq!(left.base_revision(), Some(3));
        assert_eq!(left.op_len(), 2);
    }

    #[test]
    fn mutation_batch_rejects_mismatched_base_revisions() {
        let mut left = MutationBatch::new().with_base_revision(1);
        let right = MutationBatch::new().with_base_revision(2);

        let err = left.extend(right).expect_err("mismatch should fail");
        assert!(matches!(
            err,
            StateError::MutationBaseRevisionMismatch { left: 1, right: 2 }
        ));
    }

    #[test]
    fn mutation_ops_apply_into_snapshot() {
        let mut batch = MutationBatch::new();
        batch.update::<Counter>(4);

        let mut snapshot = Snapshot {
            revision: 0,
            ext: Arc::new(SlotMap::default()),
        };

        for op in batch.ops.drain(..) {
            op.apply(&mut snapshot);
        }

        assert_eq!(snapshot.get::<Counter>().copied(), Some(4));
    }
}
