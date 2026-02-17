//! Agent execution context and activity abstractions.
//!
//! This module adds runtime-only operations onto the unified `AgentState`.

use crate::change::{AgentChangeSet, CheckpointReason};
use crate::conversation::{AgentState, Message};
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
    /// Create a new runtime context object.
    pub fn new_runtime(doc: &Value, call_id: impl Into<String>, source: impl Into<String>) -> Self {
        let mut state = Self::with_initial_state("", doc.clone());
        state.runtime.call_id = call_id.into();
        state.runtime.source = source.into();
        state.runtime.version = 0;
        state.runtime.scope_attached = false;
        state
    }

    /// Create a new runtime context object with optional activity manager.
    pub fn new_runtime_with_activity_manager(
        doc: &Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        let mut state = Self::new_runtime(doc, call_id, source);
        state.runtime.activity_manager = activity_manager;
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
        self.runtime.version = version;
        self
    }

    /// Create a runtime context object directly from a loaded thread snapshot.
    pub fn from_thread(
        thread: &crate::conversation::AgentState,
        doc: &Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        version: u64,
    ) -> Self {
        let mut state = Self::new_runtime(doc, call_id, source)
            .with_runtime_scope(Some(&thread.scope))
            .with_thread_data(thread.id.clone(), thread.messages.clone(), version);
        state.resource_id = thread.resource_id.clone();
        state.parent_thread_id = thread.parent_thread_id.clone();
        state.metadata = thread.metadata.clone();
        state
    }

    /// Create a runtime context object from a loaded thread snapshot with activity wiring.
    pub fn from_thread_with_activity_manager(
        thread: &crate::conversation::AgentState,
        doc: &Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        version: u64,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        let mut state =
            Self::new_runtime_with_activity_manager(doc, call_id, source, activity_manager)
                .with_runtime_scope(Some(&thread.scope))
                .with_thread_data(thread.id.clone(), thread.messages.clone(), version);
        state.resource_id = thread.resource_id.clone();
        state.parent_thread_id = thread.parent_thread_id.clone();
        state.metadata = thread.metadata.clone();
        state
    }

    /// Attach ephemeral scope state to a runtime context.
    #[must_use]
    pub fn with_runtime_scope(mut self, scope: Option<&ScopeState>) -> Self {
        if let Some(scope) = scope {
            self.scope = scope.clone();
            self.runtime.scope_attached = true;
        } else {
            self.scope = ScopeState::default();
            self.runtime.scope_attached = false;
        }
        self
    }

    /// Runtime thread id.
    pub fn thread_id(&self) -> &str {
        &self.id
    }

    /// Runtime storage version.
    pub fn version(&self) -> u64 {
        self.runtime.version
    }

    /// Snapshot of current messages.
    pub fn messages(&self) -> &[Arc<Message>] {
        &self.messages
    }

    /// Queue a message addition in this operation.
    pub fn add_message(&self, message: Message) {
        self.runtime
            .pending_messages
            .lock()
            .unwrap()
            .push(Arc::new(message));
    }

    /// Queue multiple messages in this operation.
    pub fn add_messages(&self, messages: impl IntoIterator<Item = Message>) {
        self.runtime
            .pending_messages
            .lock()
            .unwrap()
            .extend(messages.into_iter().map(Arc::new));
    }

    /// Current call id.
    pub fn call_id(&self) -> &str {
        &self.runtime.call_id
    }

    /// Source identifier used for tracked patches.
    pub fn source(&self) -> &str {
        &self.runtime.source
    }

    /// Borrow scope state if present.
    pub fn scope_ref(&self) -> Option<&ScopeState> {
        if self.runtime.scope_attached {
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
        T::state_ref(&self.state, base, PatchSink::new(self.runtime.ops.as_ref()))
    }

    /// Typed state reference for current call (`tool_calls.<call_id>`).
    pub fn call_state<T: State>(&self) -> T::Ref<'_> {
        let path = format!("tool_calls.{}", self.runtime.call_id);
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
            .runtime
            .activity_manager
            .as_ref()
            .map(|manager| manager.snapshot(&stream_id))
            .unwrap_or_else(|| Value::Object(Default::default()));

        ActivityContext::new(
            snapshot,
            stream_id,
            activity_type,
            self.runtime.activity_manager.clone(),
        )
    }

    /// Extract accumulated patch with context source metadata.
    pub fn take_patch(&self) -> TrackedPatch {
        let ops = std::mem::take(&mut *self.runtime.ops.lock().unwrap());
        TrackedPatch::new(Patch::with_ops(ops)).with_source(self.runtime.source.clone())
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
        let messages = std::mem::take(&mut *self.runtime.pending_messages.lock().unwrap());
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

    /// Whether state has pending runtime changes.
    pub fn has_changes(&self) -> bool {
        !self.runtime.ops.lock().unwrap().is_empty()
    }

    /// Number of queued runtime operations.
    pub fn ops_count(&self) -> usize {
        self.runtime.ops.lock().unwrap().len()
    }
}

/// Activity-scoped state context.
pub struct ActivityContext {
    doc: Value,
    stream_id: String,
    activity_type: String,
    ops: Mutex<Vec<Op>>,
    manager: Option<Arc<dyn ActivityManager>>,
}

impl ActivityContext {
    fn new(
        doc: Value,
        stream_id: String,
        activity_type: String,
        manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        Self {
            doc,
            stream_id,
            activity_type,
            ops: Mutex::new(Vec::new()),
            manager,
        }
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
}
