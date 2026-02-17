//! Agent execution context and activity abstractions.
//!
//! This module composes pure state primitives from `carve-state` with
//! invocation metadata (`call_id`, `source`) and optional activity wiring.

use carve_state::{
    parse_path, CarveError, CarveResult, Op, PatchSink, ScopeState, State, StateContext,
};
use crate::change::{AgentChangeSet as ContractAgentChangeSet, CheckpointReason};
use crate::conversation::Message;
use serde_json::Value;
use std::ops::Deref;
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

/// Checkpoint payload generated from `AgentState`.
#[derive(Debug, Clone)]
pub struct AgentChangeSet {
    /// Storage version expected by this change set.
    pub expected_version: u64,
    /// Incremental delta payload for persistence.
    pub delta: ContractAgentChangeSet,
}

impl AgentChangeSet {
    /// Build an `AgentChangeSet` from explicit delta components.
    pub fn from_parts(
        expected_version: u64,
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
        reason: CheckpointReason,
        messages: Vec<std::sync::Arc<Message>>,
        patches: Vec<carve_state::TrackedPatch>,
        snapshot: Option<Value>,
    ) -> Self {
        Self {
            expected_version,
            delta: ContractAgentChangeSet {
                run_id: run_id.into(),
                parent_run_id,
                reason,
                messages,
                patches,
                snapshot,
            },
        }
    }
}

/// Agent-facing runtime data object used by tools and plugins.
///
/// It combines:
/// - persistent state access (`StateContext`)
/// - thread-scoped runtime data (`thread_id`, `messages`, `version`)
/// - invocation metadata (`call_id`, `source`)
/// - optional ephemeral scope state (`ScopeState`)
/// - optional activity wiring
pub struct AgentState<'a> {
    state: StateContext<'a>,
    thread_id: String,
    version: u64,
    messages: Vec<Arc<Message>>,
    pending_messages: Mutex<Vec<Arc<Message>>>,
    call_id: String,
    source: String,
    scope: Option<&'a ScopeState>,
    activity_manager: Option<Arc<dyn ActivityManager>>,
}

impl<'a> AgentState<'a> {
    /// Create a new state object without activity wiring.
    pub fn new(doc: &'a Value, call_id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            state: StateContext::new(doc),
            thread_id: String::new(),
            version: 0,
            messages: Vec::new(),
            pending_messages: Mutex::new(Vec::new()),
            call_id: call_id.into(),
            source: source.into(),
            scope: None,
            activity_manager: None,
        }
    }

    /// Attach thread-scoped data.
    #[must_use]
    pub fn with_thread_data(
        mut self,
        thread_id: impl Into<String>,
        messages: Vec<Arc<Message>>,
        version: u64,
    ) -> Self {
        self.thread_id = thread_id.into();
        self.messages = messages;
        self.version = version;
        self
    }

    /// Create a context object directly from a loaded thread snapshot.
    pub fn from_thread(
        thread: &'a crate::conversation::AgentState,
        doc: &'a Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        version: u64,
    ) -> Self {
        Self::new(doc, call_id, source)
            .with_scope(Some(&thread.scope))
            .with_thread_data(thread.id.clone(), thread.messages.clone(), version)
    }

    /// Create a context object from a loaded thread snapshot with activity wiring.
    pub fn from_thread_with_activity_manager(
        thread: &'a crate::conversation::AgentState,
        doc: &'a Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        version: u64,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        Self::new_with_activity_manager(doc, call_id, source, activity_manager)
            .with_scope(Some(&thread.scope))
            .with_thread_data(thread.id.clone(), thread.messages.clone(), version)
    }

    /// Create a new state object with optional activity manager.
    pub fn new_with_activity_manager(
        doc: &'a Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        Self {
            state: StateContext::new(doc),
            thread_id: String::new(),
            version: 0,
            messages: Vec::new(),
            pending_messages: Mutex::new(Vec::new()),
            call_id: call_id.into(),
            source: source.into(),
            scope: None,
            activity_manager,
        }
    }

    /// Attach ephemeral scope state.
    #[must_use]
    pub fn with_scope(mut self, scope: Option<&'a ScopeState>) -> Self {
        self.scope = scope;
        self
    }

    /// Runtime thread id.
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Runtime storage version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Snapshot of current messages.
    pub fn messages(&self) -> &[Arc<Message>] {
        &self.messages
    }

    /// Queue a message addition in this operation.
    pub fn add_message(&self, message: Message) {
        self.pending_messages.lock().unwrap().push(Arc::new(message));
    }

    /// Queue multiple messages in this operation.
    pub fn add_messages(&self, messages: impl IntoIterator<Item = Message>) {
        self.pending_messages
            .lock()
            .unwrap()
            .extend(messages.into_iter().map(Arc::new));
    }

    /// Current call id.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// Source identifier used for tracked patches.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Borrow scope state if present.
    pub fn scope_ref(&self) -> Option<&ScopeState> {
        self.scope
    }

    /// Typed scope state accessor.
    pub fn scope<T: State>(&self) -> CarveResult<T::Ref<'_>> {
        let scope = self.scope.ok_or_else(|| {
            CarveError::invalid_operation("AgentState::scope() called but no ScopeState set")
        })?;
        Ok(scope.get::<T>())
    }

    /// Read a scope value by key.
    pub fn scope_value(&self, key: &str) -> Option<&Value> {
        self.scope.and_then(|rt| rt.value(key))
    }

    /// Typed state reference at path.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        self.state.state::<T>(path)
    }

    /// Typed state reference for current call (`tool_calls.<call_id>`).
    pub fn call_state<T: State>(&self) -> T::Ref<'_> {
        let path = format!("tool_calls.{}", self.call_id);
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
            .activity_manager
            .as_ref()
            .map(|manager| manager.snapshot(&stream_id))
            .unwrap_or_else(|| Value::Object(Default::default()));

        ActivityContext::new(
            snapshot,
            stream_id,
            activity_type,
            self.activity_manager.clone(),
        )
    }

    /// Extract accumulated patch with context source metadata.
    pub fn take_patch(&self) -> carve_state::TrackedPatch {
        self.state.take_tracked_patch(&self.source)
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
        let messages = std::mem::take(&mut *self.pending_messages.lock().unwrap());
        let patch = self.take_patch();
        let mut patches = Vec::new();
        if !patch.patch().is_empty() {
            patches.push(patch);
        }
        if messages.is_empty() && patches.is_empty() && snapshot.is_none() {
            return None;
        }
        Some(AgentChangeSet::from_parts(
            self.version,
            run_id.into(),
            parent_run_id,
            reason,
            messages,
            patches,
            snapshot,
        ))
    }

    /// Whether state has pending changes.
    pub fn has_changes(&self) -> bool {
        self.state.has_changes()
    }

    /// Number of queued operations.
    pub fn ops_count(&self) -> usize {
        self.state.ops_count()
    }

    /// Access underlying pure state context.
    pub fn state_context(&self) -> &StateContext<'a> {
        &self.state
    }
}

impl<'a> Deref for AgentState<'a> {
    type Target = StateContext<'a>;

    fn deref(&self) -> &Self::Target {
        &self.state
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
