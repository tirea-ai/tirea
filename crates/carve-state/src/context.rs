//! Context provides state access with automatic patch collection.
//!
//! Components receive a Context that allows them to read and modify state
//! through typed state references. All modifications are automatically
//! collected - no explicit `finish()` call needed.

use crate::state::{PatchSink, State};
use crate::{Op, Patch, Path, TrackedPatch};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Manager for activity state updates.
///
/// Implementations can apply activity operations and emit external events.
pub trait ActivityManager: Send + Sync {
    /// Get the current activity snapshot for a stream.
    fn snapshot(&self, stream_id: &str) -> Value;
    /// Handle an activity operation for a stream.
    fn on_activity_op(&self, stream_id: &str, activity_type: &str, op: &Op);
}

/// Context provides state access with automatic patch collection.
///
/// # Design
///
/// - Components use `state::<T>()` to get typed state references
/// - All modifications are automatically collected
/// - Framework extracts patch via `take_patch()` after execution
///
/// # Example
///
/// ```ignore
/// use carve_state::{Context, State};
/// use carve_state_derive::State;
///
/// #[derive(State)]
/// struct TodoState {
///     pub items: Vec<String>,
///     pub count: i64,
/// }
///
/// // In a tool implementation:
/// async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
///     let todos = ctx.state::<TodoState>("components.todos");
///     todos.items_push("new item");
///     todos.increment_count(1);
///
///     Ok(ToolResult::success("add_todo", json!({})))
/// }
/// // Framework automatically calls ctx.take_patch() after execution
/// ```
pub struct Context<'a> {
    doc: &'a Value,
    call_id: String,
    source: String,
    ops: Mutex<Vec<Op>>,
    activity_manager: Option<Arc<dyn ActivityManager>>,
    /// Execution control: blocked reason (if execution should be blocked).
    blocked: Mutex<Option<String>>,
    /// Execution control: pending data (arbitrary JSON for pending state).
    pending: Mutex<Option<Value>>,
}

impl<'a> Context<'a> {
    /// Create a new context.
    ///
    /// # Arguments
    ///
    /// - `doc`: The current state document (snapshot)
    /// - `call_id`: Current call ID (for call_state path)
    /// - `source`: Source identifier for patch tracking
    pub fn new(doc: &'a Value, call_id: impl Into<String>, source: impl Into<String>) -> Self {
        Self::new_with_activity_manager(doc, call_id, source, None)
    }

    /// Create a new context with an activity manager.
    ///
    /// This enables activity state updates with immediate event emission.
    pub fn new_with_activity_manager(
        doc: &'a Value,
        call_id: impl Into<String>,
        source: impl Into<String>,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> Self {
        Self {
            doc,
            call_id: call_id.into(),
            source: source.into(),
            ops: Mutex::new(Vec::new()),
            activity_manager,
            blocked: Mutex::new(None),
            pending: Mutex::new(None),
        }
    }

    /// Get the current call ID.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// Get the source identifier.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Get a typed state reference at the specified path.
    ///
    /// All modifications through the reference are automatically collected.
    ///
    /// # Arguments
    ///
    /// - `path`: Dot-separated path (e.g., "components.todos", "execution.data")
    ///   - Empty string means root
    ///
    /// # Example
    ///
    /// ```ignore
    /// let todos = ctx.state::<TodoState>("components.todos");
    /// todos.items_push("new item");  // Automatically collected
    /// ```
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        T::state_ref(self.doc, base, PatchSink::new(&self.ops))
    }

    /// Get typed state reference for current call state (syntax sugar).
    ///
    /// Equivalent to `state::<T>(&format!("tool_calls.{}", self.call_id()))`
    ///
    /// # Example
    ///
    /// ```ignore
    /// let call = ctx.call_state::<CallState>();
    /// call.set_step(1);  // Automatically collected
    /// ```
    pub fn call_state<T: State>(&self) -> T::Ref<'_> {
        let path = format!("tool_calls.{}", self.call_id);
        self.state::<T>(&path)
    }

    /// Create an activity context for a specific stream.
    ///
    /// The activity context uses its own state snapshot and emits activity updates
    /// immediately via the activity manager (if configured).
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

    /// Extract all collected operations as a tracked patch.
    ///
    /// This is called by the framework after execution.
    /// The operations are moved out (cleared), so subsequent calls return empty patches.
    pub fn take_patch(&self) -> TrackedPatch {
        let ops = std::mem::take(&mut *self.ops.lock().unwrap());
        TrackedPatch::new(Patch::with_ops(ops)).with_source(&self.source)
    }

    /// Check if any operations have been collected.
    pub fn has_changes(&self) -> bool {
        !self.ops.lock().unwrap().is_empty()
    }

    /// Get the number of operations collected.
    pub fn ops_count(&self) -> usize {
        self.ops.lock().unwrap().len()
    }

    // =========================================================================
    // Execution control methods
    // =========================================================================

    /// Set the blocked state with a reason.
    ///
    /// When blocked, subsequent execution can check this to skip processing.
    pub fn set_blocked(&self, reason: impl Into<String>) {
        *self.blocked.lock().unwrap() = Some(reason.into());
    }

    /// Check if execution is blocked.
    pub fn is_blocked(&self) -> bool {
        self.blocked.lock().unwrap().is_some()
    }

    /// Get the block reason if blocked.
    pub fn block_reason(&self) -> Option<String> {
        self.blocked.lock().unwrap().clone()
    }

    /// Clear the blocked state.
    pub fn clear_blocked(&self) {
        *self.blocked.lock().unwrap() = None;
    }

    /// Set the pending state with arbitrary data.
    ///
    /// When pending, execution should pause for user interaction.
    pub fn set_pending(&self, data: impl Into<Value>) {
        *self.pending.lock().unwrap() = Some(data.into());
    }

    /// Check if execution is pending.
    pub fn is_pending(&self) -> bool {
        self.pending.lock().unwrap().is_some()
    }

    /// Get the pending data if any.
    pub fn pending_data(&self) -> Option<Value> {
        self.pending.lock().unwrap().clone()
    }

    /// Clear the pending state.
    pub fn clear_pending(&self) {
        *self.pending.lock().unwrap() = None;
    }
}

/// Activity context that mirrors state operations with immediate event emission.
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

/// Parse a dot-separated path string into a Path.
pub fn parse_path(path: &str) -> Path {
    if path.is_empty() {
        return Path::root();
    }

    let mut result = Path::root();
    for segment in path.split('.') {
        if !segment.is_empty() {
            result = result.key(segment);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =============================================================================
    // parse_path tests
    // =============================================================================

    #[test]
    fn test_parse_path_empty() {
        let path = parse_path("");
        assert!(path.is_empty());
    }

    #[test]
    fn test_parse_path_single() {
        let path = parse_path("execution");
        assert_eq!(path.to_string(), "$.execution");
    }

    #[test]
    fn test_parse_path_nested() {
        let path = parse_path("components.my_tool");
        assert_eq!(path.to_string(), "$.components.my_tool");
    }

    #[test]
    fn test_parse_path_deep() {
        let path = parse_path("tool_calls.call_123.data");
        assert_eq!(path.to_string(), "$.tool_calls.call_123.data");
    }

    #[test]
    fn test_parse_path_leading_dot() {
        let path = parse_path(".foo.bar");
        assert_eq!(path.to_string(), "$.foo.bar");
    }

    #[test]
    fn test_parse_path_trailing_dot() {
        let path = parse_path("foo.bar.");
        assert_eq!(path.to_string(), "$.foo.bar");
    }

    #[test]
    fn test_parse_path_consecutive_dots() {
        let path = parse_path("foo..bar");
        assert_eq!(path.to_string(), "$.foo.bar");
    }

    // =============================================================================
    // Context creation tests
    // =============================================================================

    #[test]
    fn test_context_new() {
        let doc = json!({"test": 1});
        let ctx = Context::new(&doc, "call_id_123", "tool:my_tool");

        assert_eq!(ctx.call_id(), "call_id_123");
        assert_eq!(ctx.source(), "tool:my_tool");
        assert!(!ctx.has_changes());
        assert_eq!(ctx.ops_count(), 0);
    }

    #[test]
    fn test_context_empty_call_id() {
        let doc = json!({});
        let ctx = Context::new(&doc, "", "source");

        assert_eq!(ctx.call_id(), "");
    }

    #[test]
    fn test_context_unicode_identifiers() {
        let doc = json!({});
        let ctx = Context::new(&doc, "调用_123", "工具:测试");

        assert_eq!(ctx.call_id(), "调用_123");
        assert_eq!(ctx.source(), "工具:测试");
    }

    #[test]
    fn test_context_take_patch_empty() {
        let doc = json!({});
        let ctx = Context::new(&doc, "c1", "s1");

        let tracked = ctx.take_patch();

        // Empty context should produce empty patch
        assert!(tracked.patch().is_empty());
        assert_eq!(tracked.source.as_deref(), Some("s1"));
    }
}
