//! Shared test fixtures for crates that depend on `tirea-contract`.
//!
//! Gated behind the `test-support` cargo feature so production builds are
//! unaffected.  Enable via `[dev-dependencies] tirea-contract = { ..., features = ["test-support"] }`.

use crate::runtime::activity::NoOpActivityManager;
use crate::runtime::action::Action;
use crate::runtime::inference::{InferenceContext, MessagingContext};
use crate::runtime::run::FlowControl;
use crate::runtime::tool_call::ToolGate;
use crate::runtime::state::AnyStateAction;
use crate::runtime::run::TerminationReason;
use crate::runtime::tool_call::suspension::Suspension;
use crate::runtime::tool_call::{ToolDescriptor, ToolResult};
use crate::runtime::{
    PendingToolCall, Phase, RunAction, StepContext, SuspendTicket, ToolCallContext,
    ToolCallResumeMode,
};
use crate::thread::Message;
use crate::RunConfig;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tirea_state::{DocCell, Op};

pub struct TestFixture {
    pub doc: DocCell,
    pub ops: Mutex<Vec<Op>>,
    pub run_config: RunConfig,
    pub pending_messages: Mutex<Vec<Arc<Message>>>,
    pub messages: Vec<Arc<Message>>,
}

impl TestFixture {
    pub fn new() -> Self {
        Self {
            doc: DocCell::new(serde_json::json!({})),
            ops: Mutex::new(Vec::new()),
            run_config: RunConfig::default(),
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
            "test",
            "test",
            &self.run_config,
            &self.pending_messages,
            NoOpActivityManager::arc(),
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
            call_id,
            source,
            &self.run_config,
            &self.pending_messages,
            NoOpActivityManager::arc(),
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

/// Standard mock tool set with configurable third tool.
pub fn mock_tools() -> Vec<ToolDescriptor> {
    mock_tools_with("delete_file", "Delete File", "Delete a file")
}

/// Mock tool set with a custom third tool.
pub fn mock_tools_with(id: &str, display: &str, desc: &str) -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor::new("read_file", "Read File", "Read a file"),
        ToolDescriptor::new("write_file", "Write File", "Write a file"),
        ToolDescriptor::new(id, display, desc),
    ]
}

/// Build a `SuspendTicket` from a `Suspension` interaction for tests.
pub fn test_suspend_ticket(interaction: Suspension) -> SuspendTicket {
    let tool_name = interaction
        .action
        .strip_prefix("tool:")
        .unwrap_or("TestSuspend")
        .to_string();
    SuspendTicket::new(
        interaction.clone(),
        PendingToolCall::new(interaction.id, tool_name, interaction.parameters),
        ToolCallResumeMode::PassDecisionToTool,
    )
}

/// Validate and apply actions in tests using the same path as runtime.
pub fn apply_actions_for_test(
    phase: Phase,
    step: &mut StepContext<'_>,
    actions: Vec<Box<dyn Action>>,
) -> Result<(), String> {
    for action in &actions {
        action.validate(phase)?;
    }
    for action in actions {
        action.apply(step);
    }
    Ok(())
}

// =============================================================================
// Test-only Action types
//
// These exist solely for framework tests (agent-loop, orchestrator) that need
// concrete Action implementations but don't belong to any plugin crate.
// =============================================================================

/// Test action: append a line to system prompt context.
pub struct TestSystemContext(pub String);

impl Action for TestSystemContext {
    fn label(&self) -> &'static str {
        "add_system_context"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference {
            Ok(())
        } else {
            Err(format!(
                "TestSystemContext is only allowed in BeforeInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<InferenceContext>()
            .system_context
            .push(self.0);
    }
}

/// Test action: append a session context message.
pub struct TestSessionContext(pub String);

impl Action for TestSessionContext {
    fn label(&self) -> &'static str {
        "add_session_context"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference {
            Ok(())
        } else {
            Err(format!(
                "TestSessionContext is only allowed in BeforeInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<InferenceContext>()
            .session_context
            .push(self.0);
    }
}

/// Test action: block tool execution with a reason.
pub struct TestBlockTool(pub String);

impl Action for TestBlockTool {
    fn label(&self) -> &'static str {
        "block_tool"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeToolExecute {
            Ok(())
        } else {
            Err(format!(
                "TestBlockTool is only allowed in BeforeToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        if let Some(gate) = step.extensions.get_mut::<ToolGate>() {
            gate.blocked = true;
            gate.block_reason = Some(self.0);
            gate.pending = false;
            gate.suspend_ticket = None;
        }
    }
}

/// Test action: suspend tool execution with a ticket.
pub struct TestSuspendTool(pub SuspendTicket);

impl Action for TestSuspendTool {
    fn label(&self) -> &'static str {
        "suspend_tool"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeToolExecute {
            Ok(())
        } else {
            Err(format!(
                "TestSuspendTool is only allowed in BeforeToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        if let Some(gate) = step.extensions.get_mut::<ToolGate>() {
            gate.blocked = false;
            gate.block_reason = None;
            gate.pending = true;
            gate.suspend_ticket = Some(self.0);
        }
    }
}

/// Test action: exclude a tool by ID.
pub struct TestExcludeTool(pub String);

impl Action for TestExcludeTool {
    fn label(&self) -> &'static str {
        "exclude_tool"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference {
            Ok(())
        } else {
            Err(format!(
                "TestExcludeTool is only allowed in BeforeInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        let inf = step.extensions.get_or_default::<InferenceContext>();
        inf.tools.retain(|t| t.id != self.0);
    }
}

/// Test action: request run termination.
pub struct TestRequestTermination(pub TerminationReason);

impl Action for TestRequestTermination {
    fn label(&self) -> &'static str {
        "request_termination"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference || phase == Phase::AfterInference {
            Ok(())
        } else {
            Err(format!(
                "TestRequestTermination is only allowed in BeforeInference/AfterInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<FlowControl>()
            .run_action = Some(RunAction::Terminate(self.0));
    }
}

/// Test action: emit a state patch.
pub struct TestEmitStatePatch(pub AnyStateAction);

impl Action for TestEmitStatePatch {
    fn label(&self) -> &'static str {
        "emit_state_patch"
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.emit_state_action(self.0);
    }
}

/// Test action: append a system reminder after tool results.
pub struct TestSystemReminder(pub String);

impl Action for TestSystemReminder {
    fn label(&self) -> &'static str {
        "add_system_reminder"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::AfterToolExecute {
            Ok(())
        } else {
            Err(format!(
                "TestSystemReminder is only allowed in AfterToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<MessagingContext>()
            .reminders
            .push(self.0);
    }
}

/// Test action: allow tool execution (clears block/suspend).
pub struct TestAllowTool;

impl Action for TestAllowTool {
    fn label(&self) -> &'static str {
        "allow_tool"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeToolExecute {
            Ok(())
        } else {
            Err(format!(
                "TestAllowTool is only allowed in BeforeToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        if let Some(gate) = step.extensions.get_mut::<ToolGate>() {
            gate.blocked = false;
            gate.block_reason = None;
            gate.pending = false;
            gate.suspend_ticket = None;
        }
    }
}

/// Test action: override tool result directly.
pub struct TestOverrideToolResult(pub ToolResult);

impl Action for TestOverrideToolResult {
    fn label(&self) -> &'static str {
        "override_tool_result"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeToolExecute {
            Ok(())
        } else {
            Err(format!(
                "TestOverrideToolResult is only allowed in BeforeToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        if let Some(gate) = step.extensions.get_mut::<ToolGate>() {
            gate.result = Some(self.0);
        }
    }
}

/// Test action: keep only specified tools.
pub struct TestIncludeOnlyTools(pub Vec<String>);

impl Action for TestIncludeOnlyTools {
    fn label(&self) -> &'static str {
        "include_only_tools"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference {
            Ok(())
        } else {
            Err(format!(
                "TestIncludeOnlyTools is only allowed in BeforeInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        let inf = step.extensions.get_or_default::<InferenceContext>();
        inf.tools.retain(|t| self.0.iter().any(|id| id == &t.id));
    }
}

/// Test action: append a user message.
pub struct TestUserMessage(pub String);

impl Action for TestUserMessage {
    fn label(&self) -> &'static str {
        "add_user_message"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::AfterToolExecute {
            Ok(())
        } else {
            Err(format!(
                "TestUserMessage is only allowed in AfterToolExecute, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<MessagingContext>()
            .user_messages
            .push(self.0);
    }
}
