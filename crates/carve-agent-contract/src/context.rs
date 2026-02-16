//! Agent execution context and activity abstractions.
//!
//! This module keeps agent-runtime semantics (`activity`) out of `carve-state`.

use carve_state::{parse_path, CarveResult, Context as StateContext, Op, PatchSink, Runtime, State};
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

/// Agent-facing context used by tools and plugins.
///
/// It wraps `carve_state::Context` for state access and patch collection,
/// and adds optional activity wiring for runtime event emission.
pub struct Context<'a> {
    state: StateContext<'a>,
    activity_manager: Option<Arc<dyn ActivityManager>>,
}

impl<'a> Context<'a> {
    /// Create a new context without activity wiring.
    pub fn new(doc: &'a Value, call_id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            state: StateContext::new(doc, call_id, source),
            activity_manager: None,
        }
    }

    /// Create a new context with optional activity manager.
    pub fn new_with_activity_manager(
        doc: &'a Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        Self {
            state: StateContext::new(doc, call_id, source),
            activity_manager,
        }
    }

    /// Attach per-run runtime context.
    pub fn with_runtime(mut self, runtime: Option<&'a Runtime>) -> Self {
        self.state = self.state.with_runtime(runtime);
        self
    }

    /// Current call id.
    pub fn call_id(&self) -> &str {
        self.state.call_id()
    }

    /// Source identifier used for tracked patches.
    pub fn source(&self) -> &str {
        self.state.source()
    }

    /// Borrow runtime if present.
    pub fn runtime_ref(&self) -> Option<&Runtime> {
        self.state.runtime_ref()
    }

    /// Typed runtime accessor.
    pub fn runtime<T: State>(&self) -> CarveResult<T::Ref<'_>> {
        self.state.runtime::<T>()
    }

    /// Read a runtime value by key.
    pub fn runtime_value(&self, key: &str) -> Option<&Value> {
        self.state.runtime_value(key)
    }

    /// Typed state reference at path.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        self.state.state::<T>(path)
    }

    /// Typed state reference for current call (`tool_calls.<call_id>`).
    pub fn call_state<T: State>(&self) -> T::Ref<'_> {
        self.state.call_state::<T>()
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

    /// Extract accumulated patch.
    pub fn take_patch(&self) -> carve_state::TrackedPatch {
        self.state.take_patch()
    }

    /// Whether state has pending changes.
    pub fn has_changes(&self) -> bool {
        self.state.has_changes()
    }

    /// Number of queued operations.
    pub fn ops_count(&self) -> usize {
        self.state.ops_count()
    }

    /// Access underlying state context for lower-level integrations.
    pub fn state_context(&self) -> &StateContext<'a> {
        &self.state
    }
}

impl<'a> Deref for Context<'a> {
    type Target = StateContext<'a>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

/// Alias for explicit naming at call sites that prefer `AgentContext`.
pub type AgentContext<'a> = Context<'a>;

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
