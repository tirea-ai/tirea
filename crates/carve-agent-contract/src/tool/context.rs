//! Execution context types for tools and plugins.
//!
//! `ToolCallContext` provides state access, run config, and identity for tool execution.
//! It replaces direct `&Thread` usage in tool signatures, keeping the persistent
//! entity (`Thread`) invisible to tools and plugins.

use crate::runtime::activity::ActivityManager;
use crate::thread::Message;
use crate::RunConfig;
use carve_state::{
    get_at_path, parse_path, CarveResult, DocCell, Op, Patch, PatchSink, State, TrackedPatch,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Execution context for tool invocations.
///
/// Provides typed state access (read/write), run config access, identity,
/// message queuing, and activity tracking. Tools receive `&ToolCallContext`
/// instead of `&Thread`.
pub struct ToolCallContext<'a> {
    doc: &'a DocCell,
    ops: &'a Mutex<Vec<Op>>,
    run_patch: Arc<Mutex<Vec<Op>>>,
    call_id: String,
    source: String,
    run_config: &'a RunConfig,
    pending_messages: &'a Mutex<Vec<Arc<Message>>>,
    activity_manager: Option<Arc<dyn ActivityManager>>,
}

impl<'a> ToolCallContext<'a> {
    /// Create a new tool call context.
    pub fn new(
        doc: &'a DocCell,
        ops: &'a Mutex<Vec<Op>>,
        run_patch: Arc<Mutex<Vec<Op>>>,
        call_id: impl Into<String>,
        source: impl Into<String>,
        run_config: &'a RunConfig,
        pending_messages: &'a Mutex<Vec<Arc<Message>>>,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        Self {
            doc,
            ops,
            run_patch,
            call_id: call_id.into(),
            source: source.into(),
            run_config,
            pending_messages,
            activity_manager,
        }
    }

    // =========================================================================
    // Identity
    // =========================================================================

    /// Current call id (typically the `tool_call_id`).
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// Stable idempotency key for the current tool invocation.
    ///
    /// Tools should use this value when implementing idempotent side effects.
    pub fn idempotency_key(&self) -> &str {
        self.call_id()
    }

    /// Source identifier used for tracked patches.
    pub fn source(&self) -> &str {
        &self.source
    }

    // =========================================================================
    // Run Config
    // =========================================================================

    /// Borrow the run config.
    pub fn run_config(&self) -> &RunConfig {
        self.run_config
    }

    /// Typed run config accessor.
    pub fn config_state<T: State>(&self) -> CarveResult<T::Ref<'_>> {
        Ok(self.run_config.get::<T>())
    }

    /// Read a run config value by key.
    pub fn config_value(&self, key: &str) -> Option<&Value> {
        self.run_config.value(key)
    }

    // =========================================================================
    // State access
    // =========================================================================

    /// Typed state reference at path.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        let doc = self.doc;
        let hook: Arc<dyn Fn(&Op) + Send + Sync + '_> = Arc::new(|op: &Op| {
            doc.apply(op);
        });
        T::state_ref(doc, base, PatchSink::new_with_hook(self.ops, hook))
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
    /// Reads from the shared `doc`; writes go to the run overlay instead of
    /// thread ops but still update `doc` for immediate read-back.
    pub fn run_state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        let doc = self.doc;
        let hook: Arc<dyn Fn(&Op) + Send + Sync + '_> = Arc::new(|op: &Op| {
            doc.apply(op);
        });
        T::state_ref(
            doc,
            base,
            PatchSink::new_with_hook(self.run_patch.as_ref(), hook),
        )
    }

    /// Typed state reference at canonical path, writing to the run overlay.
    ///
    /// Panics if `T::PATH` is empty.
    pub fn run_state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use run_state::<T>(path) instead"
        );
        self.run_state::<T>(T::PATH)
    }

    /// Typed state reference for current call (`tool_calls.<call_id>`).
    pub fn call_state<T: State>(&self) -> T::Ref<'_> {
        let path = format!("tool_calls.{}", self.call_id);
        self.state::<T>(&path)
    }

    // =========================================================================
    // Messages
    // =========================================================================

    /// Queue a message addition in this operation.
    pub fn add_message(&self, message: Message) {
        self.pending_messages
            .lock()
            .unwrap()
            .push(Arc::new(message));
    }

    /// Queue multiple messages in this operation.
    pub fn add_messages(&self, messages: impl IntoIterator<Item = Message>) {
        self.pending_messages
            .lock()
            .unwrap()
            .extend(messages.into_iter().map(Arc::new));
    }

    // =========================================================================
    // Activity
    // =========================================================================

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

    // =========================================================================
    // State snapshot
    // =========================================================================

    /// Snapshot the current document state.
    ///
    /// Returns the current state including all write-through updates.
    /// Equivalent to `Thread::rebuild_state()` in transient contexts.
    pub fn snapshot(&self) -> Value {
        self.doc.snapshot()
    }

    /// Typed snapshot at the type's canonical path.
    ///
    /// Reads current doc state and deserializes the value at `T::PATH`.
    pub fn snapshot_of<T: State>(&self) -> CarveResult<T> {
        let val = self.doc.snapshot();
        let at = get_at_path(&val, &parse_path(T::PATH)).unwrap_or(&Value::Null);
        T::from_value(at)
    }

    /// Typed snapshot at an explicit path.
    ///
    /// Reads current doc state and deserializes the value at the given path.
    pub fn snapshot_at<T: State>(&self, path: &str) -> CarveResult<T> {
        let val = self.doc.snapshot();
        let at = get_at_path(&val, &parse_path(path)).unwrap_or(&Value::Null);
        T::from_value(at)
    }

    // =========================================================================
    // Patch extraction
    // =========================================================================

    /// Extract accumulated patch with context source metadata.
    pub fn take_patch(&self) -> TrackedPatch {
        let ops = std::mem::take(&mut *self.ops.lock().unwrap());
        TrackedPatch::new(Patch::with_ops(ops)).with_source(self.source.clone())
    }

    /// Whether state has pending transient changes.
    pub fn has_changes(&self) -> bool {
        !self.ops.lock().unwrap().is_empty()
    }

    /// Number of queued transient operations.
    pub fn ops_count(&self) -> usize {
        self.ops.lock().unwrap().len()
    }
}

/// Activity-scoped state context.
pub struct ActivityContext {
    doc: DocCell,
    stream_id: String,
    activity_type: String,
    ops: Mutex<Vec<Op>>,
    manager: Option<Arc<dyn ActivityManager>>,
}

impl ActivityContext {
    pub(crate) fn new(
        doc: Value,
        stream_id: String,
        activity_type: String,
        manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        Self {
            doc: DocCell::new(doc),
            stream_id,
            activity_type,
            ops: Mutex::new(Vec::new()),
            manager,
        }
    }

    /// Typed activity state reference at the type's canonical path.
    ///
    /// Panics if `T::PATH` is empty.
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
    /// to the activity manager (if configured). Writes are applied to the
    /// shared doc for immediate read-back.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        if let Some(manager) = self.manager.clone() {
            let stream_id = self.stream_id.clone();
            let activity_type = self.activity_type.clone();
            let doc = &self.doc;
            let hook: Arc<dyn Fn(&Op) + Send + Sync + '_> = Arc::new(move |op: &Op| {
                doc.apply(op);
                manager.on_activity_op(&stream_id, &activity_type, op);
            });
            T::state_ref(&self.doc, base, PatchSink::new_with_hook(&self.ops, hook))
        } else {
            let doc = &self.doc;
            let hook: Arc<dyn Fn(&Op) + Send + Sync + '_> = Arc::new(move |op: &Op| {
                doc.apply(op);
            });
            T::state_ref(&self.doc, base, PatchSink::new_with_hook(&self.ops, hook))
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::control::LoopControlState;
    use serde_json::json;

    fn make_ctx<'a>(
        doc: &'a DocCell,
        ops: &'a Mutex<Vec<Op>>,
        overlay: Arc<Mutex<Vec<Op>>>,
        run_config: &'a RunConfig,
        pending: &'a Mutex<Vec<Arc<Message>>>,
    ) -> ToolCallContext<'a> {
        ToolCallContext::new(doc, ops, overlay, "call-1", "test", run_config, pending, None)
    }

    #[test]
    fn test_identity() {
        let doc = DocCell::new(json!({}));
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let scope = RunConfig::default();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay, &scope, &pending);
        assert_eq!(ctx.call_id(), "call-1");
        assert_eq!(ctx.idempotency_key(), "call-1");
        assert_eq!(ctx.source(), "test");
    }

    #[test]
    fn test_scope_access() {
        let doc = DocCell::new(json!({}));
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let mut scope = RunConfig::new();
        scope.set("user_id", "u1").unwrap();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay, &scope, &pending);
        assert_eq!(ctx.config_value("user_id"), Some(&json!("u1")));
        assert_eq!(ctx.config_value("missing"), None);
    }

    #[test]
    fn test_state_of_read_write() {
        let doc = DocCell::new(
            json!({"loop_control": {"pending_interaction": null, "inference_error": null}}),
        );
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let scope = RunConfig::default();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay.clone(), &scope, &pending);

        // Write
        let ctrl = ctx.state_of::<LoopControlState>();
        ctrl.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "rate_limit".into(),
            message: "too many requests".into(),
        }));

        // Read back from same ref
        let err = ctrl.inference_error().unwrap();
        assert!(err.is_some());
        assert_eq!(err.unwrap().error_type, "rate_limit");

        // Ops captured in thread ops, not overlay
        assert!(!ops.lock().unwrap().is_empty());
        assert!(overlay.lock().unwrap().is_empty());
    }

    #[test]
    fn test_write_through_read_cross_ref() {
        let doc = DocCell::new(
            json!({"loop_control": {"pending_interaction": null, "inference_error": null}}),
        );
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let scope = RunConfig::default();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay, &scope, &pending);

        // Write via first ref
        ctx.state_of::<LoopControlState>()
            .set_inference_error(Some(crate::runtime::control::InferenceError {
                error_type: "timeout".into(),
                message: "timed out".into(),
            }));

        // Read via second ref
        let err = ctx.state_of::<LoopControlState>().inference_error().unwrap();
        assert_eq!(err.unwrap().error_type, "timeout");
    }

    #[test]
    fn test_run_state_of_writes_to_overlay() {
        let doc = DocCell::new(
            json!({"loop_control": {"pending_interaction": null, "inference_error": null}}),
        );
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let scope = RunConfig::default();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay.clone(), &scope, &pending);

        ctx.run_state_of::<LoopControlState>()
            .set_inference_error(Some(crate::runtime::control::InferenceError {
                error_type: "overridden".into(),
                message: "from overlay".into(),
            }));

        assert!(ops.lock().unwrap().is_empty(), "thread ops must be empty");
        assert!(
            !overlay.lock().unwrap().is_empty(),
            "overlay must have ops"
        );

        // Readable via state_of (write-through to shared doc)
        let err = ctx.state_of::<LoopControlState>().inference_error().unwrap();
        assert_eq!(err.unwrap().error_type, "overridden");
    }

    #[test]
    fn test_take_patch() {
        let doc = DocCell::new(
            json!({"loop_control": {"pending_interaction": null, "inference_error": null}}),
        );
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let scope = RunConfig::default();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay, &scope, &pending);

        ctx.state_of::<LoopControlState>()
            .set_inference_error(Some(crate::runtime::control::InferenceError {
                error_type: "test".into(),
                message: "test".into(),
            }));

        assert!(ctx.has_changes());
        assert!(ctx.ops_count() > 0);

        let patch = ctx.take_patch();
        assert!(!patch.patch().is_empty());
        assert_eq!(patch.source.as_deref(), Some("test"));
        assert!(!ctx.has_changes());
        assert_eq!(ctx.ops_count(), 0);
    }

    #[test]
    fn test_add_messages() {
        let doc = DocCell::new(json!({}));
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let scope = RunConfig::default();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay, &scope, &pending);

        ctx.add_message(Message::user("hello"));
        ctx.add_messages(vec![Message::assistant("hi"), Message::user("bye")]);

        assert_eq!(pending.lock().unwrap().len(), 3);
    }

    #[test]
    fn test_call_state() {
        let doc = DocCell::new(json!({"tool_calls": {}}));
        let ops = Mutex::new(Vec::new());
        let overlay = Arc::new(Mutex::new(Vec::new()));
        let scope = RunConfig::default();
        let pending = Mutex::new(Vec::new());

        let ctx = make_ctx(&doc, &ops, overlay, &scope, &pending);

        let ctrl = ctx.call_state::<LoopControlState>();
        ctrl.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "call_scoped".into(),
            message: "test".into(),
        }));

        assert!(ctx.has_changes());
    }
}
