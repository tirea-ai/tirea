use crate::context::ToolCallContext;
use crate::run_delta::RunDelta;
use crate::runtime::control::LoopControlState;
use crate::runtime::interaction::Interaction;
use crate::state::transient::ActivityManager;
use crate::state::Message;
use crate::RunConfig;
use carve_state::{
    apply_patch, apply_patches, CarveResult, DeltaTracked, DocCell, Op, Patch, State, TrackedPatch,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Run-scoped workspace that holds mutable state for a single agent run.
///
/// `RunContext` is constructed from a `Thread`'s persisted data at the start
/// of a run and accumulates messages, patches, and overlay ops as the run
/// progresses. It owns the `DocCell` (live document) and provides delta
/// extraction via `take_delta()`.
///
/// It does **not** hold the `Thread` itself — only the data needed for
/// execution. The thread identity is carried as `thread_id`.
pub struct RunContext {
    thread_id: String,
    state: Value,
    messages: DeltaTracked<Arc<Message>>,
    patches: DeltaTracked<TrackedPatch>,
    pub run_config: RunConfig,
    run_overlay: Arc<Mutex<Vec<Op>>>,
    doc: DocCell,
    version: Option<u64>,
    version_timestamp: Option<u64>,
}

impl RunContext {
    /// Build a run workspace from thread data.
    ///
    /// - `thread_id`: thread identifier (owned)
    /// - `state`: already-rebuilt state (base + patches)
    /// - `messages`: initial messages (cursor set to end — no delta)
    /// - `run_config`: per-run sealed configuration
    pub fn new(
        thread_id: impl Into<String>,
        state: Value,
        messages: Vec<Arc<Message>>,
        run_config: RunConfig,
    ) -> Self {
        let doc = DocCell::new(state.clone());
        Self {
            thread_id: thread_id.into(),
            state,
            messages: DeltaTracked::new(messages),
            patches: DeltaTracked::empty(),
            run_config,
            run_overlay: Arc::new(Mutex::new(Vec::new())),
            doc,
            version: None,
            version_timestamp: None,
        }
    }

    // =========================================================================
    // Identity
    // =========================================================================

    /// Thread identifier.
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    // =========================================================================
    // Version
    // =========================================================================

    /// Current committed version (0 if never committed).
    pub fn version(&self) -> u64 {
        self.version.unwrap_or(0)
    }

    /// Update version after a successful state commit.
    pub fn set_version(&mut self, version: u64, timestamp: Option<u64>) {
        self.version = Some(version);
        if let Some(ts) = timestamp {
            self.version_timestamp = Some(ts);
        }
    }

    /// Timestamp of the last committed version.
    pub fn version_timestamp(&self) -> Option<u64> {
        self.version_timestamp
    }

    // =========================================================================
    // Pending interaction
    // =========================================================================

    /// Read pending interaction from durable control state.
    ///
    /// This rebuilds state and navigates to `loop_control.pending_interaction`.
    pub fn pending_interaction(&self) -> Option<Interaction> {
        self.rebuild_state()
            .ok()
            .and_then(|state| {
                state
                    .get(LoopControlState::PATH)
                    .and_then(|lc| lc.get("pending_interaction"))
                    .cloned()
            })
            .and_then(|value| serde_json::from_value::<Interaction>(value).ok())
    }

    // =========================================================================
    // Messages
    // =========================================================================

    /// All messages (initial + accumulated during run).
    pub fn messages(&self) -> &[Arc<Message>] {
        self.messages.as_slice()
    }

    /// Add a single message to the run.
    pub fn add_message(&mut self, msg: Arc<Message>) {
        self.messages.push(msg);
    }

    /// Add multiple messages to the run.
    pub fn add_messages(&mut self, msgs: Vec<Arc<Message>>) {
        self.messages.extend(msgs);
    }

    // =========================================================================
    // State / Patches
    // =========================================================================

    /// The initial rebuilt state (base + thread patches, before run overlay).
    pub fn state(&self) -> &Value {
        &self.state
    }

    /// Add a tracked patch from this run.
    pub fn add_patch(&mut self, patch: TrackedPatch) {
        self.patches.push(patch);
    }

    /// Add multiple tracked patches from this run.
    pub fn add_patches(&mut self, patches: Vec<TrackedPatch>) {
        self.patches.extend(patches);
    }

    /// All patches accumulated during this run.
    pub fn patches(&self) -> &[TrackedPatch] {
        self.patches.as_slice()
    }

    // =========================================================================
    // Doc (live document)
    // =========================================================================

    /// Rebuild the live document from `state + patches + overlay`.
    ///
    /// Call this when the doc needs to reflect newly added patches.
    pub fn rebuild_doc(&mut self) {
        let patches = self.patches.as_slice();
        let base = if patches.is_empty() {
            self.state.clone()
        } else {
            apply_patches(&self.state, patches.iter().map(|p| p.patch()))
                .unwrap_or_else(|_| self.state.clone())
        };
        let overlay_ops = self.run_overlay.lock().unwrap();
        let value = if overlay_ops.is_empty() {
            base
        } else {
            apply_patch(&base, &Patch::with_ops(overlay_ops.clone())).unwrap_or(base)
        };
        self.doc = DocCell::new(value);
    }

    /// Rebuild the current run-visible state (state + patches + overlay).
    ///
    /// This is a pure computation that returns a new `Value` without
    /// touching the `DocCell`.
    pub fn rebuild_state(&self) -> CarveResult<Value> {
        let patches = self.patches.as_slice();
        let base = if patches.is_empty() {
            self.state.clone()
        } else {
            apply_patches(&self.state, patches.iter().map(|p| p.patch()))?
        };
        let overlay_ops = self.run_overlay.lock().unwrap();
        if overlay_ops.is_empty() {
            Ok(base)
        } else {
            apply_patch(&base, &Patch::with_ops(overlay_ops.clone()))
        }
    }

    /// Borrow the live document.
    pub fn doc(&self) -> &DocCell {
        &self.doc
    }

    // =========================================================================
    // Overlay
    // =========================================================================

    /// Clone the shared run overlay handle.
    pub fn run_overlay(&self) -> Arc<Mutex<Vec<Op>>> {
        self.run_overlay.clone()
    }

    // =========================================================================
    // Delta output
    // =========================================================================

    /// Extract the incremental delta (new messages + patches) since the last
    /// `take_delta()` call.
    pub fn take_delta(&mut self) -> RunDelta {
        RunDelta {
            messages: self.messages.take_delta(),
            patches: self.patches.take_delta(),
        }
    }

    /// Whether there are un-consumed messages or patches.
    pub fn has_delta(&self) -> bool {
        self.messages.has_delta() || self.patches.has_delta()
    }

    // =========================================================================
    // ToolCallContext derivation
    // =========================================================================

    /// Create a `ToolCallContext` scoped to a specific tool call.
    pub fn tool_call_context<'ctx>(
        &'ctx self,
        ops: &'ctx Mutex<Vec<Op>>,
        call_id: impl Into<String>,
        source: impl Into<String>,
        pending_messages: &'ctx Mutex<Vec<Arc<Message>>>,
        activity_manager: Option<Arc<dyn ActivityManager>>,
    ) -> ToolCallContext<'ctx> {
        ToolCallContext::new(
            &self.doc,
            ops,
            self.run_overlay.clone(),
            call_id,
            source,
            &self.run_config,
            pending_messages,
            activity_manager,
        )
    }

}

impl RunContext {
    /// Convenience constructor from a `Thread`.
    ///
    /// Rebuilds state from the thread's base state + patches, then wraps
    /// the thread's messages and run_config into a `RunContext`. Version
    /// metadata is carried over from thread metadata.
    pub fn from_thread(thread: &crate::state::Thread) -> Result<Self, carve_state::CarveError> {
        let state = thread.rebuild_state()?;
        let messages: Vec<Arc<Message>> = thread.messages.clone();
        let mut ctx = Self::new(thread.id.clone(), state, messages, thread.run_config.clone());
        if let Some(v) = thread.metadata.version {
            ctx.set_version(v, thread.metadata.version_timestamp);
        }
        Ok(ctx)
    }
}

impl std::fmt::Debug for RunContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunContext")
            .field("thread_id", &self.thread_id)
            .field("messages", &self.messages.len())
            .field("patches", &self.patches.len())
            .field("has_delta", &self.has_delta())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_state::{path, Patch};
    use serde_json::json;

    #[test]
    fn new_context_has_no_delta() {
        let msgs = vec![Arc::new(Message::user("hi"))];
        let mut ctx = RunContext::new("t-1", json!({}), msgs, RunConfig::default());
        assert!(!ctx.has_delta());
        let delta = ctx.take_delta();
        assert!(delta.is_empty());
        assert_eq!(ctx.messages().len(), 1);
    }

    #[test]
    fn add_message_creates_delta() {
        let mut ctx = RunContext::new("t-1", json!({}), vec![], RunConfig::default());
        ctx.add_message(Arc::new(Message::user("hello")));
        ctx.add_message(Arc::new(Message::assistant("hi")));
        assert!(ctx.has_delta());
        let delta = ctx.take_delta();
        assert_eq!(delta.messages.len(), 2);
        assert!(delta.patches.is_empty());
        assert!(!ctx.has_delta());
        assert_eq!(ctx.messages().len(), 2);
    }

    #[test]
    fn add_patch_creates_delta() {
        let mut ctx = RunContext::new("t-1", json!({"a": 1}), vec![], RunConfig::default());
        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("a"), json!(2))));
        ctx.add_patch(patch);
        assert!(ctx.has_delta());
        let delta = ctx.take_delta();
        assert_eq!(delta.patches.len(), 1);
        assert!(!ctx.has_delta());
    }

    #[test]
    fn rebuild_doc_reflects_patches() {
        let mut ctx = RunContext::new("t-1", json!({"counter": 0}), vec![], RunConfig::default());
        ctx.add_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        ));
        ctx.rebuild_doc();
        assert_eq!(ctx.doc().snapshot()["counter"], 5);
    }

    #[test]
    fn rebuild_state_includes_overlay() {
        let ctx = RunContext::new("t-1", json!({"counter": 0}), vec![], RunConfig::default());
        ctx.run_overlay
            .lock()
            .unwrap()
            .push(Op::set(path!("counter"), json!(99)));
        let state = ctx.rebuild_state().unwrap();
        assert_eq!(state["counter"], 99);
    }

    #[test]
    fn multiple_deltas() {
        let mut ctx = RunContext::new("t-1", json!({}), vec![], RunConfig::default());
        ctx.add_message(Arc::new(Message::user("a")));
        let d1 = ctx.take_delta();
        assert_eq!(d1.messages.len(), 1);

        ctx.add_message(Arc::new(Message::user("b")));
        ctx.add_message(Arc::new(Message::user("c")));
        let d2 = ctx.take_delta();
        assert_eq!(d2.messages.len(), 2);

        let d3 = ctx.take_delta();
        assert!(d3.is_empty());
    }

    // =========================================================================
    // Category 1: Delta extraction incremental semantics
    // =========================================================================

    /// Initial messages passed to `new()` are NOT part of the delta.
    /// Only run-added messages appear in `take_delta()`.
    #[test]
    fn initial_messages_excluded_from_delta() {
        let initial = vec![
            Arc::new(Message::user("pre-existing-1")),
            Arc::new(Message::assistant("pre-existing-2")),
        ];
        let mut ctx = RunContext::new("t-1", json!({}), initial, RunConfig::default());

        // No delta despite having 2 messages
        assert!(!ctx.has_delta());
        let delta = ctx.take_delta();
        assert!(delta.messages.is_empty());
        assert_eq!(ctx.messages().len(), 2);

        // Now add a run message — only that one appears
        ctx.add_message(Arc::new(Message::user("run-added")));
        let delta = ctx.take_delta();
        assert_eq!(delta.messages.len(), 1);
        assert_eq!(delta.messages[0].content, "run-added");
        // Total messages still include initial
        assert_eq!(ctx.messages().len(), 3);
    }

    /// All patches are delta (cursor starts at 0) — every patch added during
    /// a run is considered new.
    #[test]
    fn all_patches_are_delta() {
        let mut ctx = RunContext::new("t-1", json!({"a": 0}), vec![], RunConfig::default());
        ctx.add_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("a"), json!(1))),
        ));
        ctx.add_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("a"), json!(2))),
        ));
        let delta = ctx.take_delta();
        assert_eq!(delta.patches.len(), 2, "all run patches should be in delta");
    }

    /// Multiple take_delta calls produce non-overlapping results.
    #[test]
    fn consecutive_take_delta_non_overlapping() {
        let mut ctx = RunContext::new("t-1", json!({}), vec![], RunConfig::default());

        // Round 1: 1 message + 1 patch
        ctx.add_message(Arc::new(Message::user("m1")));
        ctx.add_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("x"), json!(1))),
        ));
        let d1 = ctx.take_delta();
        assert_eq!(d1.messages.len(), 1);
        assert_eq!(d1.patches.len(), 1);

        // Round 2: 2 messages + 1 patch (no overlap with d1)
        ctx.add_message(Arc::new(Message::user("m2")));
        ctx.add_message(Arc::new(Message::user("m3")));
        ctx.add_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("y"), json!(2))),
        ));
        let d2 = ctx.take_delta();
        assert_eq!(d2.messages.len(), 2);
        assert_eq!(d2.patches.len(), 1);

        // Round 3: nothing added
        let d3 = ctx.take_delta();
        assert!(d3.is_empty());

        // Total accumulated
        assert_eq!(ctx.messages().len(), 3);
        assert_eq!(ctx.patches().len(), 2);
    }

    // =========================================================================
    // Category 4: Run overlay not leaking to delta
    // =========================================================================

    /// Overlay ops are visible in rebuild_state but NOT in take_delta.
    #[test]
    fn overlay_visible_in_state_not_in_delta() {
        let mut ctx = RunContext::new("t-1", json!({"counter": 0}), vec![], RunConfig::default());

        // Add overlay op
        ctx.run_overlay
            .lock()
            .unwrap()
            .push(Op::set(path!("counter"), json!(42)));

        // Visible in rebuild_state
        let state = ctx.rebuild_state().unwrap();
        assert_eq!(state["counter"], 42);

        // NOT in delta (overlay ops are ephemeral)
        let delta = ctx.take_delta();
        assert!(delta.patches.is_empty(), "overlay ops must not appear in delta");
        assert!(delta.messages.is_empty());
    }

    /// Overlay ops don't appear in patches() either — they're separate.
    #[test]
    fn overlay_not_in_patches() {
        let ctx = RunContext::new("t-1", json!({"x": 0}), vec![], RunConfig::default());
        ctx.run_overlay
            .lock()
            .unwrap()
            .push(Op::set(path!("x"), json!(99)));

        assert!(ctx.patches().is_empty(), "overlay must not appear in patches()");
        // But state reflects it
        assert_eq!(ctx.rebuild_state().unwrap()["x"], 99);
    }

    /// Overlay + patches coexist: patches in delta, overlay not.
    #[test]
    fn overlay_and_patches_coexist_only_patches_in_delta() {
        let mut ctx = RunContext::new("t-1", json!({"a": 0, "b": 0}), vec![], RunConfig::default());

        // Durable patch on "a"
        ctx.add_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("a"), json!(10))),
        ));
        // Ephemeral overlay on "b"
        ctx.run_overlay
            .lock()
            .unwrap()
            .push(Op::set(path!("b"), json!(20)));

        // State shows both
        let state = ctx.rebuild_state().unwrap();
        assert_eq!(state["a"], 10);
        assert_eq!(state["b"], 20);

        // Delta only has the durable patch
        let delta = ctx.take_delta();
        assert_eq!(delta.patches.len(), 1, "only durable patches in delta");
    }

    // =========================================================================
    // Category 5: from_thread boundary conditions
    // =========================================================================

    #[test]
    fn from_thread_rebuilds_existing_patches() {
        use crate::state::Thread;

        let mut thread = Thread::with_initial_state("t-1", json!({"counter": 0}));
        thread.patches.push(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        ));

        let ctx = RunContext::from_thread(&thread).unwrap();
        // State is pre-rebuilt (includes thread patches)
        assert_eq!(ctx.state()["counter"], 5);
        // No run patches yet
        assert!(ctx.patches().is_empty());
        // rebuild_state is consistent with state()
        assert_eq!(ctx.rebuild_state().unwrap()["counter"], 5);
    }

    #[test]
    fn from_thread_carries_version_metadata() {
        use crate::state::Thread;

        let mut thread = Thread::new("t-1");
        thread.metadata.version = Some(42);
        thread.metadata.version_timestamp = Some(1700000000);

        let ctx = RunContext::from_thread(&thread).unwrap();
        assert_eq!(ctx.version(), 42);
        assert_eq!(ctx.version_timestamp(), Some(1700000000));
    }

    #[test]
    fn from_thread_broken_patch_returns_error() {
        use crate::state::Thread;

        let mut thread = Thread::with_initial_state("t-1", json!({"x": 1}));
        // Append to a non-array path — this will fail during rebuild_state
        thread.patches.push(TrackedPatch::new(
            Patch::with_ops(vec![carve_state::Op::Append {
                path: path!("x"),
                value: json!(999),
            }]),
        ));

        let result = RunContext::from_thread(&thread);
        assert!(result.is_err(), "broken patch should cause from_thread to fail");
    }

    // =========================================================================
    // Version tracking
    // =========================================================================

    #[test]
    fn version_defaults_to_zero() {
        let ctx = RunContext::new("t-1", json!({}), vec![], RunConfig::default());
        assert_eq!(ctx.version(), 0);
        assert_eq!(ctx.version_timestamp(), None);
    }

    #[test]
    fn set_version_updates_correctly() {
        let mut ctx = RunContext::new("t-1", json!({}), vec![], RunConfig::default());
        ctx.set_version(5, Some(1700000000));
        assert_eq!(ctx.version(), 5);
        assert_eq!(ctx.version_timestamp(), Some(1700000000));

        // Update again
        ctx.set_version(6, None);
        assert_eq!(ctx.version(), 6);
        // Timestamp unchanged when None passed
        assert_eq!(ctx.version_timestamp(), Some(1700000000));
    }
}
