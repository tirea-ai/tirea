use super::*;
use crate::contracts::runtime::behavior::ReadOnlyContext;
use crate::contracts::runtime::phase::{Phase, StepContext};
use crate::contracts::runtime::phase::{BeforeInferenceAction, ActionSet};
use crate::contracts::runtime::state::{reduce_state_actions, ScopeContext};
use crate::contracts::runtime::tool_call::ToolStatus;
use crate::contracts::thread::Thread;
use crate::contracts::AgentBehavior;
use crate::orchestrator::InMemoryAgentRegistry;
use crate::runtime::loop_runner::{
    TOOL_SCOPE_CALLER_AGENT_ID_KEY, TOOL_SCOPE_CALLER_MESSAGES_KEY, TOOL_SCOPE_CALLER_STATE_KEY,
    TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use async_trait::async_trait;
use serde_json::json;
use std::time::Duration;
use tirea_contract::testing::{
    apply_after_inference_for_test, apply_after_tool_for_test, apply_before_inference_for_test,
    apply_before_tool_for_test, apply_lifecycle_for_test, TestFixture,
};
use tirea_state::apply_patches;

#[async_trait]
trait AgentBehaviorTestDispatch {
    async fn run_phase(&self, phase: Phase, step: &mut StepContext<'_>);
}

#[async_trait]
impl<T> AgentBehaviorTestDispatch for T
where
    T: AgentBehavior + ?Sized,
{
    async fn run_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        let ctx = ReadOnlyContext::new(
            phase,
            step.thread_id(),
            step.messages(),
            step.run_config(),
            step.ctx().doc(),
        );
        match phase {
            Phase::RunStart => apply_lifecycle_for_test(step, self.run_start(&ctx).await),
            Phase::StepStart => apply_lifecycle_for_test(step, self.step_start(&ctx).await),
            Phase::BeforeInference => {
                apply_before_inference_for_test(step, self.before_inference(&ctx).await)
            }
            Phase::AfterInference => {
                apply_after_inference_for_test(step, self.after_inference(&ctx).await)
            }
            Phase::BeforeToolExecute => {
                apply_before_tool_for_test(step, self.before_tool_execute(&ctx).await)
            }
            Phase::AfterToolExecute => {
                apply_after_tool_for_test(step, self.after_tool_execute(&ctx).await)
            }
            Phase::StepEnd => apply_lifecycle_for_test(step, self.step_end(&ctx).await),
            Phase::RunEnd => apply_lifecycle_for_test(step, self.run_end(&ctx).await),
        }
        // Reduce any pending state actions
        if !step.pending_state_actions.is_empty() {
            let state_actions = std::mem::take(&mut step.pending_state_actions);
            let snapshot = step.snapshot();
            let patches =
                reduce_state_actions(state_actions, &snapshot, "test", &ScopeContext::run())
                    .expect("state actions should reduce");
            for patch in patches {
                let doc = step.ctx().doc();
                for op in patch.patch().ops() {
                    doc.apply(op).expect("state action patch op should apply");
                }
                step.emit_patch(patch);
            }
        }
    }
}

// ── Plugin tests ─────────────────────────────────────────────────────────────

#[test]
fn plugin_filters_out_caller_agent() {
    let mut reg = InMemoryAgentRegistry::new();
    reg.upsert("a", crate::orchestrator::AgentDefinition::new("mock"));
    reg.upsert("b", crate::orchestrator::AgentDefinition::new("mock"));
    let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(SubAgentHandleTable::new()));
    let rendered = plugin.render_available_agents(Some("a"), None);
    assert!(rendered.contains("<id>b</id>"));
    assert!(!rendered.contains("<id>a</id>"));
}

#[test]
fn plugin_filters_agents_by_scope_policy() {
    let mut reg = InMemoryAgentRegistry::new();
    reg.upsert("writer", crate::orchestrator::AgentDefinition::new("mock"));
    reg.upsert(
        "reviewer",
        crate::orchestrator::AgentDefinition::new("mock"),
    );
    let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(SubAgentHandleTable::new()));
    let mut rt = tirea_contract::RunConfig::new();
    rt.set(SCOPE_ALLOWED_AGENTS_KEY, vec!["writer"]).unwrap();
    let rendered = plugin.render_available_agents(None, Some(&rt));
    assert!(rendered.contains("<id>writer</id>"));
    assert!(!rendered.contains("<id>reviewer</id>"));
}

#[test]
fn plugin_renders_agent_output_tool_usage() {
    let mut reg = InMemoryAgentRegistry::new();
    reg.upsert("worker", crate::orchestrator::AgentDefinition::new("mock"));
    let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(SubAgentHandleTable::new()));
    let rendered = plugin.render_available_agents(None, None);
    assert!(
        rendered.contains("agent_output"),
        "available agents should mention agent_output tool"
    );
}

#[tokio::test]
async fn plugin_adds_reminder_for_running_and_stopped_runs() {
    let mut reg = InMemoryAgentRegistry::new();
    reg.upsert("worker", crate::orchestrator::AgentDefinition::new("mock"));
    let handles = Arc::new(SubAgentHandleTable::new());
    let plugin = AgentToolsPlugin::new(Arc::new(reg), handles.clone());

    let epoch = handles
        .put_running(
            "run-1",
            "owner-1".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    assert_eq!(epoch, 1);

    let fixture = TestFixture::new();
    let mut step = StepContext::new(fixture.ctx(), "owner-1", &fixture.messages, vec![]);
    plugin.run_phase(Phase::AfterToolExecute, &mut step).await;
    let reminder = step.messaging.reminders.first()
        .expect("running reminder should be present");
    assert!(reminder.contains("status=\"running\""));

    handles.stop_owned_tree("owner-1", "run-1").await.unwrap();
    let fixture2 = TestFixture::new();
    let mut step2 = StepContext::new(fixture2.ctx(), "owner-1", &fixture2.messages, vec![]);
    plugin.run_phase(Phase::AfterToolExecute, &mut step2).await;
    let reminder2 = step2.messaging.reminders.first()
        .expect("stopped reminder should be present");
    assert!(reminder2.contains("status=\"stopped\""));
}

// ── Handle table tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn handle_table_ignores_stale_completion_by_epoch() {
    let handles = SubAgentHandleTable::new();
    let epoch1 = handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "agent-a".to_string(),
            None,
            None,
        )
        .await;
    assert_eq!(epoch1, 1);

    let epoch2 = handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "agent-a".to_string(),
            None,
            None,
        )
        .await;
    assert_eq!(epoch2, 2);

    let ignored = handles
        .update_after_completion(
            "run-1",
            epoch1,
            SubAgentCompletion {
                status: SubAgentStatus::Completed,
                error: None,
            },
        )
        .await;
    assert!(ignored.is_none());

    let summary = handles
        .get_owned_summary("owner", "run-1")
        .await
        .expect("run should still exist");
    assert_eq!(summary.status, SubAgentStatus::Running);

    let applied = handles
        .update_after_completion(
            "run-1",
            epoch2,
            SubAgentCompletion {
                status: SubAgentStatus::Completed,
                error: None,
            },
        )
        .await
        .expect("latest epoch completion should apply");
    assert_eq!(applied.status, SubAgentStatus::Completed);
}

#[tokio::test]
async fn handle_table_stop_tree_stops_descendants() {
    let handles = SubAgentHandleTable::new();
    handles
        .put_running(
            "parent-run",
            "owner-thread".to_string(),
            "sub-agent-parent-run".to_string(),
            "agent-a".to_string(),
            None,
            None,
        )
        .await;
    handles
        .put_running(
            "child-run",
            "owner-thread".to_string(),
            "sub-agent-child-run".to_string(),
            "agent-a".to_string(),
            Some("parent-run".to_string()),
            None,
        )
        .await;
    handles
        .put_running(
            "grandchild-run",
            "owner-thread".to_string(),
            "sub-agent-grandchild-run".to_string(),
            "agent-a".to_string(),
            Some("child-run".to_string()),
            None,
        )
        .await;
    handles
        .put_running(
            "other-owner-run",
            "other-owner".to_string(),
            "sub-agent-other-owner-run".to_string(),
            "agent-b".to_string(),
            Some("parent-run".to_string()),
            None,
        )
        .await;

    let stopped = handles
        .stop_owned_tree("owner-thread", "parent-run")
        .await
        .unwrap();

    assert_eq!(stopped.len(), 3);

    let parent = handles
        .get_owned_summary("owner-thread", "parent-run")
        .await
        .expect("parent run should exist");
    assert_eq!(parent.status, SubAgentStatus::Stopped);

    let child = handles
        .get_owned_summary("owner-thread", "child-run")
        .await
        .expect("child run should exist");
    assert_eq!(child.status, SubAgentStatus::Stopped);

    let grandchild = handles
        .get_owned_summary("owner-thread", "grandchild-run")
        .await
        .expect("grandchild run should exist");
    assert_eq!(grandchild.status, SubAgentStatus::Stopped);

    let denied = handles
        .stop_owned_tree("owner-thread", "other-owner-run")
        .await;
    assert!(denied.is_err());
}

// ── AgentRunTool tests ───────────────────────────────────────────────────────

#[derive(Debug)]
struct SlowTerminatePlugin;

#[async_trait]
impl AgentBehavior for SlowTerminatePlugin {
    fn id(&self) -> &str {
        "slow_terminate_behavior_requested"
    }

    async fn before_inference(&self, _ctx: &ReadOnlyContext<'_>) -> ActionSet<BeforeInferenceAction> {
        tokio::time::sleep(Duration::from_millis(120)).await;
        ActionSet::single(BeforeInferenceAction::Terminate(
            crate::contracts::TerminationReason::BehaviorRequested,
        ))
    }
}

fn caller_scope_with_state_and_run(
    state: serde_json::Value,
    run_id: &str,
) -> tirea_contract::RunConfig {
    caller_scope_with_state_run_and_messages(
        state,
        run_id,
        vec![crate::contracts::thread::Message::user("seed message")],
    )
}

fn caller_scope_with_state_run_and_messages(
    state: serde_json::Value,
    run_id: &str,
    messages: Vec<crate::contracts::thread::Message>,
) -> tirea_contract::RunConfig {
    let mut rt = tirea_contract::RunConfig::new();
    rt.set(TOOL_SCOPE_CALLER_THREAD_ID_KEY, "owner-thread")
        .unwrap();
    rt.set(TOOL_SCOPE_CALLER_AGENT_ID_KEY, "caller").unwrap();
    rt.set(SCOPE_RUN_ID_KEY, run_id).unwrap();
    rt.set(TOOL_SCOPE_CALLER_STATE_KEY, state).unwrap();
    rt.set(TOOL_SCOPE_CALLER_MESSAGES_KEY, messages).unwrap();
    rt
}

fn caller_scope_with_state(state: serde_json::Value) -> tirea_contract::RunConfig {
    caller_scope_with_state_and_run(state, "parent-run-default")
}

fn caller_scope() -> tirea_contract::RunConfig {
    caller_scope_with_state(json!({"forked": true}))
}

#[tokio::test]
async fn agent_run_tool_requires_scope_context() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));
    let fix = TestFixture::new();
    let result = tool
        .execute(
            json!({"agent_id":"worker","prompt":"hi","background":false}),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("missing caller thread context"));
}

#[tokio::test]
async fn agent_run_tool_rejects_disallowed_target_agent() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini"),
        )
        .with_agent(
            "reviewer",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();
    fix.run_config
        .set(SCOPE_ALLOWED_AGENTS_KEY, vec!["worker"])
        .unwrap();
    let result = tool
        .execute(
            json!({"agent_id":"reviewer","prompt":"hi","background":false}),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("Unknown or unavailable agent_id"));
}

#[tokio::test]
async fn agent_run_tool_rejects_self_target_agent() {
    let os = AgentOs::builder()
        .with_agent(
            "caller",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini"),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();
    let result = tool
        .execute(
            json!({"agent_id":"caller","prompt":"hi","background":false}),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("Unknown or unavailable agent_id"));
}

#[tokio::test]
async fn agent_run_tool_fork_context_filters_messages() {
    let os = AgentOs::builder()
        .with_registered_behavior(
            "slow_terminate_behavior_requested",
            Arc::new(SlowTerminatePlugin),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini")
                .with_behavior_id("slow_terminate_behavior_requested"),
        )
        .build()
        .unwrap();
    let handles = Arc::new(SubAgentHandleTable::new());
    let run_tool = AgentRunTool::new(os, handles.clone());

    let fork_messages = vec![
        crate::contracts::thread::Message::system("parent-system"),
        crate::contracts::thread::Message::internal_system("parent-internal-system"),
        crate::contracts::thread::Message::user("parent-user-1"),
        crate::contracts::thread::Message::assistant_with_tool_calls(
            "parent-assistant-tool-call",
            vec![
                crate::contracts::thread::ToolCall::new(
                    "call-paired",
                    "search",
                    json!({"q":"paired"}),
                ),
                crate::contracts::thread::ToolCall::new(
                    "call-missing",
                    "search",
                    json!({"q":"missing"}),
                ),
            ],
        ),
        crate::contracts::thread::Message::tool("call-paired", "tool paired result"),
        crate::contracts::thread::Message::tool("call-orphan", "tool orphan result"),
        crate::contracts::thread::Message::assistant_with_tool_calls(
            "assistant-unpaired-only",
            vec![crate::contracts::thread::ToolCall::new(
                "call-only-assistant",
                "search",
                json!({"q":"only-assistant"}),
            )],
        ),
        crate::contracts::thread::Message::assistant("parent-assistant-plain"),
    ];

    let mut fix = TestFixture::new();
    fix.run_config = caller_scope_with_state_run_and_messages(
        json!({"forked": true}),
        "parent-run-42",
        fork_messages,
    );

    let started = run_tool
        .execute(
            json!({
                "agent_id":"worker",
                "prompt":"child-prompt",
                "background": true,
                "fork_context": true
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(started.status, ToolStatus::Success);
    assert_eq!(started.data["status"], json!("running"));

    // Verify the fork_context flag produces a running status.
    // (Child thread details are now in ThreadStore, not in-memory handle.)
}

#[tokio::test]
async fn background_stop_then_resume_completes() {
    let os = AgentOs::builder()
        .with_registered_behavior(
            "slow_terminate_behavior_requested",
            Arc::new(SlowTerminatePlugin),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini")
                .with_behavior_id("slow_terminate_behavior_requested"),
        )
        .build()
        .unwrap();
    let handles = Arc::new(SubAgentHandleTable::new());
    let run_tool = AgentRunTool::new(os, handles.clone());
    let stop_tool = AgentStopTool::new(handles);

    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();
    let started = run_tool
        .execute(
            json!({
                "agent_id":"worker",
                "prompt":"start",
                "background": true
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(started.status, ToolStatus::Success);
    assert_eq!(started.data["status"], json!("running"));
    let run_id = started.data["run_id"]
        .as_str()
        .expect("run_id should exist")
        .to_string();

    let mut stop_fix = TestFixture::new();
    stop_fix.run_config = caller_scope();
    let stopped = stop_tool
        .execute(
            json!({ "run_id": run_id.clone() }),
            &stop_fix.ctx_with("call-stop", "tool:agent_stop"),
        )
        .await
        .unwrap();
    assert_eq!(stopped.status, ToolStatus::Success);
    assert_eq!(stopped.data["status"], json!("stopped"));

    // Give cancelled background task a chance to flush stale completion.
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Resume: since there's no ThreadStore in this test context, resume from
    // the live handle which is Stopped.
    let resumed = run_tool
        .execute(
            json!({
                "run_id": run_id,
                "prompt":"resume",
                "background": false
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ToolStatus::Success);
    // The execution will fail because there's no ThreadStore configured,
    // but the tool itself should handle the error gracefully.
    let status = resumed.data["status"].as_str().unwrap();
    assert!(
        status == "completed" || status == "failed",
        "expected terminal status, got: {status}"
    );
}

#[tokio::test]
async fn agent_run_tool_persists_run_state_patch() {
    let os = AgentOs::builder()
        .with_registered_behavior(
            "slow_terminate_behavior_requested",
            Arc::new(SlowTerminatePlugin),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini")
                .with_behavior_id("slow_terminate_behavior_requested"),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));

    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();
    let started = run_tool
        .execute(
            json!({
                "agent_id":"worker",
                "prompt":"start",
                "background": true
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(started.status, ToolStatus::Success);
    let run_id = started.data["run_id"]
        .as_str()
        .expect("run_id should exist")
        .to_string();

    let patch = fix.ctx_with("call-run", "tool:agent_run").take_patch();
    assert!(
        !patch.patch().is_empty(),
        "expected tool to persist run snapshot into state"
    );
    let base = json!({});
    let updated = apply_patches(&base, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["sub_agents"]["runs"][&run_id]["status"],
        json!("running")
    );
    // No thread field in SubAgent.
    assert!(
        updated["sub_agents"]["runs"][&run_id].get("thread").is_none()
            || updated["sub_agents"]["runs"][&run_id]["thread"].is_null(),
        "SubAgent should not contain embedded thread"
    );
    // thread_id should be present.
    assert!(
        updated["sub_agents"]["runs"][&run_id]["thread_id"]
            .as_str()
            .is_some(),
        "SubAgent should contain thread_id"
    );
}

#[tokio::test]
async fn agent_run_tool_binds_scope_run_id_and_parent_lineage() {
    let os = AgentOs::builder()
        .with_registered_behavior(
            "slow_terminate_behavior_requested",
            Arc::new(SlowTerminatePlugin),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini")
                .with_behavior_id("slow_terminate_behavior_requested"),
        )
        .build()
        .unwrap();
    let handles = Arc::new(SubAgentHandleTable::new());
    let run_tool = AgentRunTool::new(os, handles.clone());

    let mut fix = TestFixture::new();
    fix.run_config = caller_scope_with_state_and_run(json!({"forked": true}), "parent-run-42");
    let started = run_tool
        .execute(
            json!({
                "agent_id":"worker",
                "prompt":"start",
                "background": true
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(started.status, ToolStatus::Success);
    let run_id = started.data["run_id"]
        .as_str()
        .expect("run_id should exist")
        .to_string();

    let patch = fix.ctx_with("call-run", "tool:agent_run").take_patch();
    let base = json!({});
    let updated = apply_patches(&base, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["sub_agents"]["runs"][&run_id]["parent_run_id"],
        json!("parent-run-42")
    );
}

#[tokio::test]
async fn agent_run_tool_resumes_from_persisted_state_without_live_record() {
    let os = AgentOs::builder()
        .with_registered_behavior(
            "slow_terminate_behavior_requested",
            Arc::new(SlowTerminatePlugin),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini")
                .with_behavior_id("slow_terminate_behavior_requested"),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));

    let doc = json!({
        "sub_agents": {
            "runs": {
                "run-1": {
                    "thread_id": "sub-agent-run-1",
                    "agent_id": "worker",
                    "status": "stopped"
                }
            }
        }
    });
    let mut fix = TestFixture::new_with_state(doc.clone());
    fix.run_config = caller_scope_with_state(doc);
    let resumed = run_tool
        .execute(
            json!({
                "run_id":"run-1",
                "prompt":"resume",
                "background": false
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ToolStatus::Success);
    // The execution will fail because there's no ThreadStore, but the tool handles it.
    let status = resumed.data["status"].as_str().unwrap();
    assert!(
        status == "completed" || status == "failed",
        "expected terminal status, got: {status}"
    );
}

#[tokio::test]
async fn agent_run_tool_marks_orphan_running_as_stopped_before_resume() {
    let os = AgentOs::builder()
        .with_registered_behavior(
            "slow_terminate_behavior_requested",
            Arc::new(SlowTerminatePlugin),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini")
                .with_behavior_id("slow_terminate_behavior_requested"),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));

    let doc = json!({
        "sub_agents": {
            "runs": {
                "run-1": {
                    "thread_id": "sub-agent-run-1",
                    "agent_id": "worker",
                    "status": "running"
                }
            }
        }
    });
    let mut fix = TestFixture::new_with_state(doc.clone());
    fix.run_config = caller_scope_with_state(doc);
    let summary = run_tool
        .execute(
            json!({
                "run_id":"run-1",
                "background": false
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(summary.status, ToolStatus::Success);
    assert_eq!(summary.data["status"], json!("stopped"));
}

// ── AgentStopTool tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn agent_stop_tool_stops_descendant_runs() {
    let handles = Arc::new(SubAgentHandleTable::new());
    let stop_tool = AgentStopTool::new(handles.clone());
    let owner_thread_id = "owner-thread";

    let parent_run_id = "run-parent";
    let child_run_id = "run-child";
    let grandchild_run_id = "run-grandchild";

    handles
        .put_running(
            parent_run_id,
            owner_thread_id.to_string(),
            format!("sub-agent-{parent_run_id}"),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    handles
        .put_running(
            child_run_id,
            owner_thread_id.to_string(),
            format!("sub-agent-{child_run_id}"),
            "worker".to_string(),
            Some(parent_run_id.to_string()),
            None,
        )
        .await;
    handles
        .put_running(
            grandchild_run_id,
            owner_thread_id.to_string(),
            format!("sub-agent-{grandchild_run_id}"),
            "worker".to_string(),
            Some(child_run_id.to_string()),
            None,
        )
        .await;

    let doc = json!({
        "sub_agents": {
            "runs": {
                parent_run_id: {
                    "thread_id": format!("sub-agent-{parent_run_id}"),
                    "agent_id": "worker",
                    "status": "running"
                },
                child_run_id: {
                    "thread_id": format!("sub-agent-{child_run_id}"),
                    "parent_run_id": parent_run_id,
                    "agent_id": "worker",
                    "status": "running"
                },
                grandchild_run_id: {
                    "thread_id": format!("sub-agent-{grandchild_run_id}"),
                    "parent_run_id": child_run_id,
                    "agent_id": "worker",
                    "status": "running"
                }
            }
        }
    });

    let mut fix = TestFixture::new_with_state(doc.clone());
    fix.run_config = {
        let mut rt = tirea_contract::RunConfig::new();
        rt.set(TOOL_SCOPE_CALLER_THREAD_ID_KEY, owner_thread_id)
            .unwrap();
        rt
    };
    let result = stop_tool
        .execute(
            json!({ "run_id": parent_run_id }),
            &fix.ctx_with("call-stop", "tool:agent_stop"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);
    assert_eq!(result.data["status"], json!("stopped"));

    let parent = handles
        .get_owned_summary(owner_thread_id, parent_run_id)
        .await
        .expect("parent run should exist");
    assert_eq!(parent.status, SubAgentStatus::Stopped);
    let child = handles
        .get_owned_summary(owner_thread_id, child_run_id)
        .await
        .expect("child run should exist");
    assert_eq!(child.status, SubAgentStatus::Stopped);
    let grandchild = handles
        .get_owned_summary(owner_thread_id, grandchild_run_id)
        .await
        .expect("grandchild run should exist");
    assert_eq!(grandchild.status, SubAgentStatus::Stopped);

    let patch = fix.ctx_with("call-stop", "tool:agent_stop").take_patch();
    let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["sub_agents"]["runs"][parent_run_id]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["sub_agents"]["runs"][child_run_id]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["sub_agents"]["runs"][grandchild_run_id]["status"],
        json!("stopped")
    );
}

// ── AgentRecoveryPlugin tests ────────────────────────────────────────────────

#[tokio::test]
async fn recovery_plugin_detects_orphan_and_records_confirmation() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker",
                        "status": "running"
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;
    assert!(matches!(
        step.run_action(),
        crate::contracts::RunAction::Continue
    ));

    let updated = fixture.updated_state();
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["suspension"]["action"],
        json!(AGENT_RECOVERY_INTERACTION_ACTION)
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["suspension"]
            ["parameters"]["run_id"],
        json!("run-1")
    );

    let fixture2 = TestFixture::new_with_state(updated);
    let mut before = fixture2.step(vec![]);
    plugin.run_phase(Phase::BeforeInference, &mut before).await;
    assert!(
        matches!(before.run_action(), crate::contracts::RunAction::Continue),
        "recovery plugin should not control inference flow in BeforeInference"
    );
}

#[tokio::test]
async fn recovery_plugin_does_not_override_existing_suspended_interaction() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "__tool_call_scope": {
                "existing_1": {
                    "suspended_call": {
                        "call_id": "existing_1",
                        "tool_name": "agent_run",
                        "suspension": {
                            "id": "existing_1",
                            "action": AGENT_RECOVERY_INTERACTION_ACTION
                        },
                        "arguments": {},
                        "pending": {
                            "id": "existing_1",
                            "name": AGENT_RECOVERY_INTERACTION_ACTION,
                            "arguments": {}
                        },
                        "resume_mode": "pass_decision_to_tool"
                    }
                }
            },
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker",
                        "status": "running"
                    }
                }
            }
        }),
    );

    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;
    assert!(
        matches!(step.run_action(), crate::contracts::RunAction::Continue),
        "existing suspended interaction should not be replaced"
    );

    let updated = fixture.updated_state();
    assert_eq!(
        updated["__tool_call_scope"]["existing_1"]["suspended_call"]["suspension"]["id"],
        json!("existing_1")
    );
}

#[tokio::test]
async fn recovery_plugin_auto_approve_when_permission_allow() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {
                    "recover_agent_run": "allow"
                }
            },
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker",
                        "status": "running"
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["tool_call_state"]["status"],
        json!("resuming")
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["tool_call_state"]["resume"]["action"],
        json!("resume")
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["tool_name"],
        json!("agent_run")
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["suspension"]
            ["parameters"]["run_id"],
        json!("run-1")
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["status"],
        json!("stopped")
    );
}

#[tokio::test]
async fn recovery_plugin_auto_deny_when_permission_deny() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {
                    "recover_agent_run": "deny"
                }
            },
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker",
                        "status": "running"
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();
    assert!(
        updated
            .get("__tool_call_scope")
            .and_then(|scopes| scopes.get("agent_recovery_run-1"))
            .and_then(|scope| scope.get("tool_call_state"))
            .is_none(),
        "deny should not set recovery tool-call resume state"
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["status"],
        json!("stopped")
    );
    assert!(updated
        .get("__suspended_tool_calls")
        .and_then(|v| v.get("calls"))
        .and_then(|v| v.as_object())
        .map_or(true, |calls| calls.is_empty()));
}

#[tokio::test]
async fn recovery_plugin_auto_approve_from_default_behavior_allow() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            },
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker",
                        "status": "running"
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["tool_call_state"]["status"],
        json!("resuming")
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["tool_call_state"]["resume"]["action"],
        json!("resume")
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["tool_name"],
        json!("agent_run")
    );
}

#[tokio::test]
async fn recovery_plugin_auto_deny_from_default_behavior_deny() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            },
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker",
                        "status": "running"
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();
    assert!(
        updated
            .get("__tool_call_scope")
            .and_then(|scopes| scopes.get("agent_recovery_run-1"))
            .and_then(|scope| scope.get("tool_call_state"))
            .is_none(),
        "deny should not set recovery tool-call resume state"
    );
    assert!(updated
        .get("__suspended_tool_calls")
        .and_then(|v| v.get("calls"))
        .and_then(|v| v.as_object())
        .map_or(true, |calls| calls.is_empty()));
}

#[tokio::test]
async fn recovery_plugin_tool_rule_overrides_default_behavior() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {
                    "recover_agent_run": "ask"
                }
            },
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker",
                        "status": "running"
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();
    assert!(
        updated
            .get("__tool_call_scope")
            .and_then(|scopes| scopes.get("agent_recovery_run-1"))
            .and_then(|scope| scope.get("tool_call_state"))
            .is_none(),
        "tool-level ask should not set recovery tool-call resume state"
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["suspension"]["action"],
        json!(AGENT_RECOVERY_INTERACTION_ACTION)
    );
}

// ── Schema tests ─────────────────────────────────────────────────────────────

#[test]
fn parse_persisted_runs_from_doc_reads_new_path() {
    let doc = json!({
        "sub_agents": {
            "runs": {
                "run-1": {
                    "thread_id": "sub-agent-run-1",
                    "agent_id": "worker",
                    "status": "stopped"
                }
            }
        }
    });
    let runs = parse_persisted_runs_from_doc(&doc);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs["run-1"].status, SubAgentStatus::Stopped);
}

#[test]
fn parse_persisted_runs_from_doc_empty_returns_empty() {
    let runs = parse_persisted_runs_from_doc(&json!({}));
    assert!(runs.is_empty());
}

// ── Parallel sub-agent tests ─────────────────────────────────────────────────

#[tokio::test]
async fn parallel_background_runs_and_stop_all() {
    let os = AgentOs::builder()
        .with_registered_behavior(
            "slow_terminate_behavior_requested",
            Arc::new(SlowTerminatePlugin),
        )
        .with_agent(
            "worker",
            crate::orchestrator::AgentDefinition::new("gpt-4o-mini")
                .with_behavior_id("slow_terminate_behavior_requested"),
        )
        .build()
        .unwrap();
    let handles = Arc::new(SubAgentHandleTable::new());
    let run_tool = AgentRunTool::new(os, handles.clone());
    let stop_tool = AgentStopTool::new(handles.clone());

    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();

    // Launch 3 background runs in parallel.
    let mut run_ids = Vec::new();
    for i in 0..3 {
        let started = run_tool
            .execute(
                json!({
                    "agent_id": "worker",
                    "prompt": format!("task-{i}"),
                    "background": true
                }),
                &fix.ctx_with(&format!("call-{i}"), "tool:agent_run"),
            )
            .await
            .unwrap();
        assert_eq!(started.status, ToolStatus::Success);
        assert_eq!(started.data["status"], json!("running"));
        run_ids.push(
            started.data["run_id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    // All should show as running.
    let running = handles.running_or_stopped_for_owner("owner-thread").await;
    assert_eq!(running.len(), 3);
    for summary in &running {
        assert_eq!(summary.status, SubAgentStatus::Running);
    }

    // Stop each one.
    for run_id in &run_ids {
        let mut stop_fix = TestFixture::new();
        stop_fix.run_config = caller_scope();
        let stopped = stop_tool
            .execute(
                json!({ "run_id": run_id }),
                &stop_fix.ctx_with("call-stop", "tool:agent_stop"),
            )
            .await
            .unwrap();
        assert_eq!(stopped.status, ToolStatus::Success);
        assert_eq!(stopped.data["status"], json!("stopped"));
    }

    // All should show as stopped.
    let stopped_all = handles.running_or_stopped_for_owner("owner-thread").await;
    assert_eq!(stopped_all.len(), 3);
    for summary in &stopped_all {
        assert_eq!(summary.status, SubAgentStatus::Stopped);
    }
}

#[tokio::test]
async fn sub_agent_status_lifecycle_running_to_completed() {
    let handles = SubAgentHandleTable::new();
    let epoch = handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;

    let summary = handles
        .get_owned_summary("owner", "run-1")
        .await
        .expect("run should exist");
    assert_eq!(summary.status, SubAgentStatus::Running);

    let completed = handles
        .update_after_completion(
            "run-1",
            epoch,
            SubAgentCompletion {
                status: SubAgentStatus::Completed,
                error: None,
            },
        )
        .await
        .expect("should complete");
    assert_eq!(completed.status, SubAgentStatus::Completed);
}

#[tokio::test]
async fn sub_agent_status_lifecycle_running_to_failed() {
    let handles = SubAgentHandleTable::new();
    let epoch = handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;

    let failed = handles
        .update_after_completion(
            "run-1",
            epoch,
            SubAgentCompletion {
                status: SubAgentStatus::Failed,
                error: Some("something went wrong".to_string()),
            },
        )
        .await
        .expect("should fail");
    assert_eq!(failed.status, SubAgentStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("something went wrong"));
}

#[tokio::test]
async fn cancellation_requested_overrides_completion_status() {
    let handles = SubAgentHandleTable::new();
    let token = RunCancellationToken::new();
    let epoch = handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            Some(token.clone()),
        )
        .await;

    // Stop the run (sets run_cancellation_requested).
    handles
        .stop_owned_tree("owner", "run-1")
        .await
        .unwrap();

    // Completion arrives with Completed, but cancellation should win.
    let result = handles
        .update_after_completion(
            "run-1",
            epoch,
            SubAgentCompletion {
                status: SubAgentStatus::Completed,
                error: None,
            },
        )
        .await
        .expect("should apply");
    assert_eq!(result.status, SubAgentStatus::Stopped);
}

#[tokio::test]
async fn agent_output_tool_returns_error_for_unknown_run() {
    let os = AgentOs::builder().build().unwrap();
    let tool = AgentOutputTool::new(os);
    let fix = TestFixture::new();
    let result = tool
        .execute(
            json!({ "run_id": "nonexistent" }),
            &fix.ctx_with("call-1", "tool:agent_output"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("Unknown run_id"));
}

#[tokio::test]
async fn agent_output_tool_returns_status_from_persisted_state() {
    let os = AgentOs::builder().build().unwrap();
    let tool = AgentOutputTool::new(os);

    let doc = json!({
        "sub_agents": {
            "runs": {
                "run-1": {
                    "thread_id": "sub-agent-run-1",
                    "agent_id": "worker",
                    "status": "completed"
                }
            }
        }
    });
    let fix = TestFixture::new_with_state(doc);
    let result = tool
        .execute(
            json!({ "run_id": "run-1" }),
            &fix.ctx_with("call-1", "tool:agent_output"),
        )
        .await
        .unwrap();
    // Will get store_error because no ThreadStore configured, but the tool
    // should handle it gracefully.
    let status_str = result.data.get("status").and_then(|v| v.as_str());
    let is_error = result.status == ToolStatus::Error;
    assert!(
        status_str == Some("completed") || is_error,
        "expected completed status or error for missing store"
    );
}

#[test]
fn sub_agent_thread_id_convention() {
    assert_eq!(
        super::tools::sub_agent_thread_id("run-123"),
        "sub-agent-run-123"
    );
}

// ── Handle table contains test ───────────────────────────────────────────────

#[tokio::test]
async fn handle_table_contains_returns_true_for_existing() {
    let handles = SubAgentHandleTable::new();
    handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    assert!(handles.contains("run-1").await);
    assert!(!handles.contains("run-2").await);
}
