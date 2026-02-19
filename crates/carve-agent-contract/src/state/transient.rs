//! Transient context operations attached to `AgentState`.
//!
//! This module keeps `AgentState`-centric transient behaviors colocated with the
//! state model (typed state access, checkpoint building, activity state wiring).

use crate::state::{AgentChangeSet, CheckpointReason};
use crate::state::{AgentState, Message};
use carve_state::{
    parse_path, CarveError, CarveResult, Op, Patch, PatchSink, ScopeState, State, TrackedPatch,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Manager for activity state updates.
///
/// Implementations keep per-stream activity state and may emit external events.
pub trait ActivityManager: Send + Sync {
    /// Get the current activity snapshot for a stream.
    fn snapshot(&self, stream_id: &str) -> Value;

    /// Handle an activity operation for a stream.
    fn on_activity_op(&self, stream_id: &str, activity_type: &str, op: &Op);
}

impl AgentState {
    /// Create a new transient context object.
    pub fn new_transient(
        doc: &Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        let mut state = Self::with_initial_state("", doc.clone());
        state.transient.call_id = call_id.into();
        state.transient.source = source.into();
        state.transient.version = 0;
        state.transient.scope_attached = false;
        state
    }

    /// Create a new transient context object with optional activity manager.
    pub fn new_transient_with_activity_manager(
        doc: &Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        let mut state = Self::new_transient(doc, call_id, source);
        state.transient.activity_manager = activity_manager;
        state
    }

    /// Attach thread-scoped data.
    #[must_use]
    pub fn with_thread_data(
        mut self,
        thread_id: impl Into<String>,
        messages: Vec<Arc<Message>>,
        version: u64,
    ) -> Self {
        self.id = thread_id.into();
        self.messages = messages;
        self.transient.version = version;
        self
    }

    /// Create a transient context object directly from a loaded thread snapshot.
    pub fn from_thread(
        thread: &AgentState,
        doc: &Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        version: u64,
    ) -> Self {
        let mut state = Self::new_transient(doc, call_id, source)
            .with_transient_scope(Some(&thread.scope))
            .with_thread_data(thread.id.clone(), thread.messages.clone(), version);
        state.resource_id = thread.resource_id.clone();
        state.parent_thread_id = thread.parent_thread_id.clone();
        state.metadata = thread.metadata.clone();
        state.transient.run_overlay = thread.transient.run_overlay.clone();
        state
    }

    /// Create a transient context object from a loaded thread snapshot with activity wiring.
    pub fn from_thread_with_activity_manager(
        thread: &AgentState,
        doc: &Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        version: u64,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        let mut state =
            Self::new_transient_with_activity_manager(doc, call_id, source, activity_manager)
                .with_transient_scope(Some(&thread.scope))
                .with_thread_data(thread.id.clone(), thread.messages.clone(), version);
        state.resource_id = thread.resource_id.clone();
        state.parent_thread_id = thread.parent_thread_id.clone();
        state.metadata = thread.metadata.clone();
        state.transient.run_overlay = thread.transient.run_overlay.clone();
        state
    }

    /// Attach ephemeral scope state to a transient context.
    #[must_use]
    pub fn with_transient_scope(mut self, scope: Option<&ScopeState>) -> Self {
        if let Some(scope) = scope {
            self.scope = scope.clone();
            self.transient.scope_attached = true;
        } else {
            self.scope = ScopeState::default();
            self.transient.scope_attached = false;
        }
        self
    }

    /// Thread id.
    pub fn thread_id(&self) -> &str {
        &self.id
    }

    /// Storage version in transient context.
    pub fn version(&self) -> u64 {
        self.transient.version
    }

    /// Snapshot of current messages.
    pub fn messages(&self) -> &[Arc<Message>] {
        &self.messages
    }

    /// Queue a message addition in this operation.
    pub fn add_message(&self, message: Message) {
        self.transient
            .pending_messages
            .lock()
            .unwrap()
            .push(Arc::new(message));
    }

    /// Queue multiple messages in this operation.
    pub fn add_messages(&self, messages: impl IntoIterator<Item = Message>) {
        self.transient
            .pending_messages
            .lock()
            .unwrap()
            .extend(messages.into_iter().map(Arc::new));
    }

    /// Current call id.
    pub fn call_id(&self) -> &str {
        &self.transient.call_id
    }

    /// Stable idempotency key for the current tool invocation.
    ///
    /// The loop sets this to the current `tool_call_id`.
    /// Tools should use this value when implementing idempotent side effects.
    pub fn idempotency_key(&self) -> &str {
        self.call_id()
    }

    /// Source identifier used for tracked patches.
    pub fn source(&self) -> &str {
        &self.transient.source
    }

    /// Borrow scope state if present.
    pub fn scope_ref(&self) -> Option<&ScopeState> {
        if self.transient.scope_attached {
            Some(&self.scope)
        } else {
            None
        }
    }

    /// Typed scope state accessor.
    pub fn scope<T: State>(&self) -> CarveResult<T::Ref<'_>> {
        let scope = self.scope_ref().ok_or_else(|| {
            CarveError::invalid_operation("AgentState::scope() called but no ScopeState set")
        })?;
        Ok(scope.get::<T>())
    }

    /// Read a scope value by key.
    pub fn scope_value(&self, key: &str) -> Option<&Value> {
        self.scope_ref().and_then(|rt| rt.value(key))
    }

    /// Typed state reference at path.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        T::state_ref(
            &self.state,
            base,
            PatchSink::new(self.transient.ops.as_ref()),
        )
    }

    /// Typed state reference at the type's canonical path.
    ///
    /// Panics if `T::PATH` is empty (no bound path via `#[carve(path = "...")]`).
    pub fn state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use state::<T>(path) instead"
        );
        self.state::<T>(T::PATH)
    }

    /// Typed state reference that writes to the run overlay (not persisted).
    ///
    /// Reads from the current doc; writes go to the run overlay instead of
    /// thread ops. The overlay is discarded when the run ends.
    pub fn override_state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        T::state_ref(
            &self.state,
            base,
            PatchSink::new(self.transient.run_overlay.as_ref()),
        )
    }

    /// Typed state reference at canonical path, writing to the run overlay.
    ///
    /// Panics if `T::PATH` is empty.
    pub fn override_state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use override_state::<T>(path) instead"
        );
        self.override_state::<T>(T::PATH)
    }

    /// Typed state reference for current call (`tool_calls.<call_id>`).
    pub fn call_state<T: State>(&self) -> T::Ref<'_> {
        let path = format!("tool_calls.{}", self.transient.call_id);
        self.state::<T>(&path)
    }

    /// Create an activity context for a stream/type pair.
    pub fn activity(
        &self,
        stream_id: impl Into<String>,
        activity_type: impl Into<String>,
    ) -> ActivityContext {
        let stream_id = stream_id.into();
        let activity_type = activity_type.into();
        let snapshot = self
            .transient
            .activity_manager
            .as_ref()
            .map(|manager| manager.snapshot(&stream_id))
            .unwrap_or_else(|| Value::Object(Default::default()));

        ActivityContext::new(
            snapshot,
            stream_id,
            activity_type,
            self.transient.activity_manager.clone(),
            Some(self.transient.run_overlay.clone()),
        )
    }

    /// Extract accumulated patch with context source metadata.
    pub fn take_patch(&self) -> TrackedPatch {
        let ops = std::mem::take(&mut *self.transient.ops.lock().unwrap());
        TrackedPatch::new(Patch::with_ops(ops)).with_source(self.transient.source.clone())
    }

    /// Build and drain a checkpoint change set.
    ///
    /// Returns `None` if there are no new messages/patches and `snapshot` is `None`.
    pub fn checkpoint(
        &self,
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
        reason: CheckpointReason,
        snapshot: Option<Value>,
    ) -> Option<AgentChangeSet> {
        let messages = std::mem::take(&mut *self.transient.pending_messages.lock().unwrap());
        let patch = self.take_patch();
        let mut patches = Vec::new();
        if !patch.patch().is_empty() {
            patches.push(patch);
        }
        if messages.is_empty() && patches.is_empty() && snapshot.is_none() {
            return None;
        }
        Some(AgentChangeSet::from_parts(
            run_id.into(),
            parent_run_id,
            reason,
            messages,
            patches,
            snapshot,
        ))
    }

    /// Whether state has pending transient changes.
    pub fn has_changes(&self) -> bool {
        !self.transient.ops.lock().unwrap().is_empty()
    }

    /// Number of queued transient operations.
    pub fn ops_count(&self) -> usize {
        self.transient.ops.lock().unwrap().len()
    }
}

/// Activity-scoped state context.
pub struct ActivityContext {
    doc: Value,
    stream_id: String,
    activity_type: String,
    ops: Mutex<Vec<Op>>,
    manager: Option<Arc<dyn ActivityManager>>,
    run_overlay: Option<Arc<Mutex<Vec<Op>>>>,
}

impl ActivityContext {
    fn new(
        doc: Value,
        stream_id: String,
        activity_type: String,
        manager: Option<Arc<dyn ActivityManager>>,
        run_overlay: Option<Arc<Mutex<Vec<Op>>>>,
    ) -> Self {
        Self {
            doc,
            stream_id,
            activity_type,
            ops: Mutex::new(Vec::new()),
            manager,
            run_overlay,
        }
    }

    /// Typed activity state reference at the type's canonical path.
    ///
    /// Panics if `T::PATH` is empty (no bound path via `#[carve(path = "...")]`).
    pub fn state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use state::<T>(path) instead"
        );
        self.state::<T>(T::PATH)
    }

    /// Get a typed activity state reference at the specified path.
    ///
    /// All modifications are automatically collected and immediately reported
    /// to the activity manager (if configured).
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        if let Some(manager) = self.manager.clone() {
            let stream_id = self.stream_id.clone();
            let activity_type = self.activity_type.clone();
            let hook = Arc::new(move |op: &Op| {
                manager.on_activity_op(&stream_id, &activity_type, op);
            });
            T::state_ref(&self.doc, base, PatchSink::new_with_hook(&self.ops, hook))
        } else {
            T::state_ref(&self.doc, base, PatchSink::new(&self.ops))
        }
    }

    /// Typed state reference that writes to the run overlay (not persisted).
    pub fn override_state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let overlay = self
            .run_overlay
            .as_ref()
            .expect("override_state called without run overlay");
        T::state_ref(&self.doc, parse_path(path), PatchSink::new(overlay.as_ref()))
    }

    /// Typed state reference at canonical path, writing to the run overlay.
    pub fn override_state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use override_state::<T>(path) instead"
        );
        self.override_state::<T>(T::PATH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::control::LoopControlState;
    use carve_state::{path, Op};
    use serde_json::json;

    #[test]
    fn test_override_state_of_writes_to_overlay() {
        let doc = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let ctx = AgentState::new_transient(&doc, "call-1", "test");

        // Write via override — should go to run_overlay, not ops
        let ctrl = ctx.override_state_of::<LoopControlState>();
        ctrl.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "rate_limit".into(),
            message: "too many requests".into(),
        }));

        assert!(
            ctx.transient.ops.lock().unwrap().is_empty(),
            "thread ops must remain empty after override write"
        );
        assert!(
            !ctx.transient.run_overlay.lock().unwrap().is_empty(),
            "overlay must contain the override op"
        );
    }

    #[test]
    fn test_from_thread_shares_overlay() {
        let thread = AgentState::new("t-1");
        // Pre-populate overlay on the thread
        thread
            .transient
            .run_overlay
            .lock()
            .unwrap()
            .push(Op::set(path!("counter"), json!(99)));

        let doc = json!({"counter": 0});
        let ctx = AgentState::from_thread(&thread, &doc, "call-1", "test", 1);

        // The transient context should share the same Arc
        assert!(
            Arc::ptr_eq(&ctx.transient.run_overlay, &thread.transient.run_overlay),
            "from_thread must share the overlay Arc"
        );

        // Writes through the ctx should be visible from thread's overlay
        ctx.transient
            .run_overlay
            .lock()
            .unwrap()
            .push(Op::set(path!("name"), json!("bob")));
        assert_eq!(
            thread.transient.run_overlay.lock().unwrap().len(),
            2,
            "overlay writes must be visible across shared Arc"
        );
    }

    #[test]
    fn test_state_of_reads_see_overlay_via_rebuild() {
        let initial = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let thread = AgentState::with_initial_state("t-1", initial);

        // Write overlay ops directly
        thread.transient.run_overlay.lock().unwrap().push(Op::set(
            path!("loop_control", "inference_error"),
            json!({"type": "timeout", "message": "timed out"}),
        ));

        // rebuild_state should include the overlay
        let rebuilt = thread.rebuild_state().unwrap();
        assert_eq!(
            rebuilt["loop_control"]["inference_error"]["type"], "timeout",
            "rebuild_state must include overlay values"
        );

        // rebuild_thread_state should NOT include it
        let thread_state = thread.rebuild_thread_state().unwrap();
        assert!(
            thread_state["loop_control"]["inference_error"].is_null(),
            "rebuild_thread_state must exclude overlay"
        );
    }
}
