use crate::thread::Message;
use carve_state::TrackedPatch;
use std::sync::Arc;

/// Incremental output from a run step — the new messages and patches
/// accumulated since the last `take_delta()`.
///
/// This replaces the previous `PendingDelta` with a cleaner name that
/// reflects its role as run-scoped output rather than a buffer on the
/// persisted entity.
#[derive(Debug, Clone, Default)]
pub struct RunDelta {
    pub messages: Vec<Arc<Message>>,
    pub patches: Vec<TrackedPatch>,
}

impl RunDelta {
    /// Returns true if there are no new messages or patches.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.patches.is_empty()
    }
}
