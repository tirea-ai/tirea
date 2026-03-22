//! ActiveRunRegistry: tracks active runs, at most one per thread.

use std::collections::HashMap;
use std::sync::RwLock;

use super::RunHandle;

pub(crate) use crate::runtime::cancellation::CancellationToken;

pub(super) struct RunEntry {
    #[allow(dead_code)]
    pub(super) run_id: String,
    #[allow(dead_code)]
    pub(super) agent_id: String,
    pub(super) handle: RunHandle,
}

/// Tracks active runs. At most one active run per thread.
pub(crate) struct ActiveRunRegistry {
    by_thread_id: RwLock<HashMap<String, RunEntry>>,
}

impl ActiveRunRegistry {
    pub(crate) fn new() -> Self {
        Self {
            by_thread_id: RwLock::new(HashMap::new()),
        }
    }

    /// Atomically insert if the thread has no active run. Returns `false` if already occupied.
    pub(super) fn try_insert(&self, thread_id: String, entry: RunEntry) -> bool {
        use std::collections::hash_map::Entry;
        let mut map = self
            .by_thread_id
            .write()
            .expect("active runs lock poisoned");
        match map.entry(thread_id) {
            Entry::Occupied(_) => false,
            Entry::Vacant(v) => {
                v.insert(entry);
                true
            }
        }
    }

    pub(super) fn remove(&self, thread_id: &str) {
        self.by_thread_id
            .write()
            .expect("active runs lock poisoned")
            .remove(thread_id);
    }

    pub(super) fn get_handle(&self, thread_id: &str) -> Option<RunHandle> {
        self.by_thread_id
            .read()
            .expect("active runs lock poisoned")
            .get(thread_id)
            .map(|e| e.handle.clone())
    }
}
