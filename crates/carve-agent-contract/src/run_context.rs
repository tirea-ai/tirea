use crate::context::ToolCallContext;
use crate::run_delta::RunDelta;
use crate::state::transient::ActivityManager;
use crate::state::Message;
use crate::RunConfig;
use carve_state::{
    apply_patch, apply_patches, CarveResult, DeltaTracked, DocCell, Op, Patch, TrackedPatch,
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
pub struct RunContext<'a> {
    thread_id: &'a str,
    state: Value,
    messages: DeltaTracked<Arc<Message>>,
    patches: DeltaTracked<TrackedPatch>,
    pub run_config: RunConfig,
    run_overlay: Arc<Mutex<Vec<Op>>>,
    doc: DocCell,
}

impl<'a> RunContext<'a> {
    /// Build a run workspace from thread data.
    ///
    /// - `thread_id`: borrowed thread identifier
    /// - `state`: already-rebuilt state (base + patches)
    /// - `messages`: initial messages (cursor set to end — no delta)
    /// - `run_config`: per-run sealed configuration
    pub fn new(
        thread_id: &'a str,
        state: Value,
        messages: Vec<Arc<Message>>,
        run_config: RunConfig,
    ) -> Self {
        let doc = DocCell::new(state.clone());
        Self {
            thread_id,
            state,
            messages: DeltaTracked::new(messages),
            patches: DeltaTracked::empty(),
            run_config,
            run_overlay: Arc::new(Mutex::new(Vec::new())),
            doc,
        }
    }

    // =========================================================================
    // Identity
    // =========================================================================

    /// Thread identifier (borrowed from the owning thread).
    pub fn thread_id(&self) -> &str {
        self.thread_id
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

impl std::fmt::Debug for RunContext<'_> {
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
}
