//! Shared mutable document for write-through-read state access.
//!
//! `DocCell` wraps a `Mutex<Value>` so that state writes (via `PatchSink` hooks)
//! immediately update the document and subsequent reads see the latest values.

use crate::apply::apply_op;
use crate::Op;
use serde_json::Value;
use std::sync::{Mutex, MutexGuard};

/// Shared mutable document for write-through-read state access.
///
/// All state reads lock briefly to fetch the current value, and all writes
/// apply the operation in-place so the next read sees the update.
pub struct DocCell(Mutex<Value>);

impl DocCell {
    /// Create a new `DocCell` with the given initial value.
    pub fn new(value: Value) -> Self {
        Self(Mutex::new(value))
    }

    /// Acquire a read lock on the document.
    ///
    /// The returned guard dereferences to `&Value`. Callers should clone
    /// any needed data before dropping the guard.
    #[inline]
    pub fn get(&self) -> MutexGuard<'_, Value> {
        self.0.lock().unwrap()
    }

    /// Apply a single operation to the document in-place.
    ///
    /// This panics if applying the operation fails. In `StateContext` write-through
    /// flows, an invalid operation is a programming error and should fail fast.
    pub fn apply(&self, op: &Op) {
        apply_op(&mut self.0.lock().unwrap(), op)
            .expect("DocCell::apply failed while applying collected operation");
    }

    /// Consume the `DocCell` and return the inner value.
    pub fn into_inner(self) -> Value {
        self.0.into_inner().unwrap()
    }

    /// Clone the current document value.
    pub fn snapshot(&self) -> Value {
        self.get().clone()
    }
}

impl Default for DocCell {
    fn default() -> Self {
        Self::new(Value::Object(Default::default()))
    }
}

impl Clone for DocCell {
    fn clone(&self) -> Self {
        Self::new(self.snapshot())
    }
}

impl std::fmt::Debug for DocCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DocCell").field(&"<Value>").finish()
    }
}
