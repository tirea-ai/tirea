//! Shared test fixtures for crates that depend on `carve-agent-contract`.
//!
//! Gated behind the `test-support` cargo feature so production builds are
//! unaffected.  Enable via `[dev-dependencies] carve-agent-contract = { ..., features = ["test-support"] }`.

use crate::context::ToolCallContext;
use crate::runtime::phase::StepContext;
use crate::state::Message;
use crate::tool::ToolDescriptor;
use carve_state::{DocCell, Op, ScopeState};
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub struct TestFixture {
    pub doc: DocCell,
    pub ops: Mutex<Vec<Op>>,
    pub overlay: Arc<Mutex<Vec<Op>>>,
    pub scope: ScopeState,
    pub pending_messages: Mutex<Vec<Arc<Message>>>,
    pub messages: Vec<Arc<Message>>,
}

impl TestFixture {
    pub fn new() -> Self {
        Self {
            doc: DocCell::new(serde_json::json!({})),
            ops: Mutex::new(Vec::new()),
            overlay: Arc::new(Mutex::new(Vec::new())),
            scope: ScopeState::default(),
            pending_messages: Mutex::new(Vec::new()),
            messages: Vec::new(),
        }
    }

    pub fn new_with_state(state: Value) -> Self {
        Self {
            doc: DocCell::new(state),
            ..Self::new()
        }
    }

    pub fn ctx(&self) -> ToolCallContext<'_> {
        ToolCallContext::new(
            &self.doc,
            &self.ops,
            self.overlay.clone(),
            "test",
            "test",
            &self.scope,
            &self.pending_messages,
            None,
        )
    }

    pub fn ctx_with(
        &self,
        call_id: impl Into<String>,
        source: impl Into<String>,
    ) -> ToolCallContext<'_> {
        ToolCallContext::new(
            &self.doc,
            &self.ops,
            self.overlay.clone(),
            call_id,
            source,
            &self.scope,
            &self.pending_messages,
            None,
        )
    }

    pub fn step(&self, tools: Vec<ToolDescriptor>) -> StepContext<'_> {
        StepContext::new(self.ctx(), "test-thread", &self.messages, tools)
    }

    pub fn has_changes(&self) -> bool {
        !self.ops.lock().unwrap().is_empty()
    }

    pub fn updated_state(&self) -> Value {
        self.doc.snapshot()
    }
}

impl Default for TestFixture {
    fn default() -> Self {
        Self::new()
    }
}
