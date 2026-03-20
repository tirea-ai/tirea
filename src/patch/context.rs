use crate::foundation::*;

#[derive(Clone)]
pub struct PhaseContext {
    pub phase: Phase,
    pub snapshot: Snapshot,
}

impl PhaseContext {
    pub fn get<K: StateSlot>(&self) -> Option<&K::Value> {
        self.snapshot.get::<K>()
    }
}
