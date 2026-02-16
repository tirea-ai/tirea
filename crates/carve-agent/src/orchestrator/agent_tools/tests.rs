use super::*;
use crate::contracts::conversation::Thread;
use crate::contracts::traits::tool::ToolStatus;
use crate::orchestrator::InMemoryAgentRegistry;
use crate::runtime::loop_runner::{
    TOOL_RUNTIME_CALLER_AGENT_ID_KEY, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
use async_trait::async_trait;
use carve_state::apply_patches;
use serde_json::json;
use std::time::Duration;

#[test]
fn plugin_filters_out_caller_agent() {
    let mut reg = InMemoryAgentRegistry::new();
    reg.upsert(
        "a",
        crate::runtime::loop_runner::AgentDefinition::new("mock"),
    );
    reg.upsert(
        "b",
        crate::runtime::loop_runner::AgentDefinition::new("mock"),
    );
    let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(AgentRunManager::new()));
    let rendered = plugin.render_available_agents(Some("a"), None);
    assert!(rendered.contains("<id>b</id>"));
    assert!(!rendered.contains("<id>a</id>"));
}

#[test]
fn plugin_filters_agents_by_runtime_policy() {
    let mut reg = InMemoryAgentRegistry::new();
    reg.upsert(
        "writer",
        crate::runtime::loop_runner::AgentDefinition::new("mock"),
    );
    reg.upsert(
        "reviewer",
        crate::runtime::loop_runner::AgentDefinition::new("mock"),
    );
    let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(AgentRunManager::new()));
    let mut rt = carve_state::ScopeState::new();
    rt.set(RUNTIME_ALLOWED_AGENTS_KEY, vec!["writer"]).unwrap();
    let rendered = plugin.render_available_agents(None, Some(&rt));
    assert!(rendered.contains("<id>writer</id>"));
    assert!(!rendered.contains("<id>reviewer</id>"));
}

#[tokio::test]
async fn plugin_adds_reminder_for_running_and_stopped_runs() {
    let doc = json!({});
    let ctx = Context::new(&doc, "test", "test");
    let mut reg = InMemoryAgentRegistry::new();
    reg.upsert(
        "worker",
        crate::runtime::loop_runner::AgentDefinition::new("mock"),
    );
    let manager = Arc::new(AgentRunManager::new());
    let plugin = AgentToolsPlugin::new(Arc::new(reg), manager.clone());

    let epoch = manager
        .put_running(
            "run-1",
            "owner-1".to_string(),
            "worker".to_string(),
            None,
            Thread::new("child-1"),
            None,
        )
        .await;
    assert_eq!(epoch, 1);

    let owner = Thread::new("owner-1");
    let mut step = StepContext::new(&owner, vec![]);
    plugin
        .on_phase(Phase::AfterToolExecute, &mut step, &ctx)
        .await;
    let reminder = step
        .system_reminders
        .first()
        .expect("running reminder should be present");
    assert!(reminder.contains("status=\"running\""));

    manager.stop_owned_tree("owner-1", "run-1").await.unwrap();
    let mut step2 = StepContext::new(&owner, vec![]);
    plugin
        .on_phase(Phase::AfterToolExecute, &mut step2, &ctx)
        .await;
    let reminder2 = step2
        .system_reminders
        .first()
        .expect("stopped reminder should be present");
    assert!(reminder2.contains("status=\"stopped\""));
}

#[tokio::test]
async fn manager_ignores_stale_completion_by_epoch() {
    let manager = AgentRunManager::new();
    let epoch1 = manager
        .put_running(
            "run-1",
            "owner".to_string(),
            "agent-a".to_string(),
            None,
            Thread::new("s-1"),
            None,
        )
        .await;
    assert_eq!(epoch1, 1);

    let epoch2 = manager
        .put_running(
            "run-1",
            "owner".to_string(),
            "agent-a".to_string(),
            None,
            Thread::new("s-2"),
            None,
        )
        .await;
    assert_eq!(epoch2, 2);

    let ignored = manager
        .update_after_completion(
            "run-1",
            epoch1,
            AgentRunCompletion {
                thread: Thread::new("old"),
                status: AgentRunStatus::Completed,
                assistant: Some("old".to_string()),
                error: None,
            },
        )
        .await;
    assert!(ignored.is_none());

    let summary = manager
        .get_owned_summary("owner", "run-1")
        .await
        .expect("run should still exist");
    assert_eq!(summary.status, AgentRunStatus::Running);

    let applied = manager
        .update_after_completion(
            "run-1",
            epoch2,
            AgentRunCompletion {
                thread: Thread::new("new"),
                status: AgentRunStatus::Completed,
                assistant: Some("new".to_string()),
                error: None,
            },
        )
        .await
        .expect("latest epoch completion should apply");
    assert_eq!(applied.status, AgentRunStatus::Completed);
    assert_eq!(applied.assistant.as_deref(), Some("new"));
}

#[tokio::test]
async fn agent_run_tool_requires_runtime_context() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));
    let doc = json!({});
    let ctx = crate::contracts::context::Context::new(&doc, "call-1", "tool:agent_run");
    let result = tool
        .execute(
            json!({"agent_id":"worker","prompt":"hi","background":false}),
            &ctx,
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
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini"),
        )
        .with_agent(
            "reviewer",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));
    let doc = json!({});
    let mut rt = caller_runtime();
    rt.set(RUNTIME_ALLOWED_AGENTS_KEY, vec!["worker"]).unwrap();
    let ctx = crate::contracts::context::Context::new(&doc, "call-1", "tool:agent_run").with_scope(Some(&rt));
    let result = tool
        .execute(
            json!({"agent_id":"reviewer","prompt":"hi","background":false}),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("Unknown or unavailable agent_id"));
}

#[derive(Debug)]
struct SlowSkipPlugin;

#[async_trait]
impl AgentPlugin for SlowSkipPlugin {
    fn id(&self) -> &str {
        "slow_skip"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
        if phase == Phase::BeforeInference {
            tokio::time::sleep(Duration::from_millis(120)).await;
            step.skip_inference = true;
        }
    }
}

fn caller_runtime_with_state_and_run(
    state: serde_json::Value,
    run_id: &str,
) -> carve_state::ScopeState {
    let mut rt = carve_state::ScopeState::new();
    rt.set(TOOL_RUNTIME_CALLER_THREAD_ID_KEY, "owner-thread")
        .unwrap();
    rt.set(TOOL_RUNTIME_CALLER_AGENT_ID_KEY, "caller").unwrap();
    rt.set(RUNTIME_RUN_ID_KEY, run_id).unwrap();
    rt.set(TOOL_RUNTIME_CALLER_STATE_KEY, state).unwrap();
    rt.set(
        TOOL_RUNTIME_CALLER_MESSAGES_KEY,
        vec![crate::contracts::conversation::Message::user(
            "seed message",
        )],
    )
    .unwrap();
    rt
}

fn caller_runtime_with_state(state: serde_json::Value) -> carve_state::ScopeState {
    caller_runtime_with_state_and_run(state, "parent-run-default")
}

fn caller_runtime() -> carve_state::ScopeState {
    caller_runtime_with_state(json!({"forked": true}))
}

#[tokio::test]
async fn background_stop_then_resume_completes() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                .with_plugin(Arc::new(SlowSkipPlugin)),
        )
        .build()
        .unwrap();
    let manager = Arc::new(AgentRunManager::new());
    let run_tool = AgentRunTool::new(os, manager.clone());
    let stop_tool = AgentStopTool::new(manager);

    let doc = json!({});
    let rt = caller_runtime();
    let ctx = crate::contracts::context::Context::new(&doc, "call-run", "tool:agent_run").with_scope(Some(&rt));
    let started = run_tool
        .execute(
            json!({
                "agent_id":"worker",
                "prompt":"start",
                "background": true
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(started.status, ToolStatus::Success);
    assert_eq!(started.data["status"], json!("running"));
    let run_id = started.data["run_id"]
        .as_str()
        .expect("run_id should exist")
        .to_string();

    let stop_ctx =
        crate::contracts::context::Context::new(&doc, "call-stop", "tool:agent_stop").with_scope(Some(&rt));
    let stopped = stop_tool
        .execute(json!({ "run_id": run_id.clone() }), &stop_ctx)
        .await
        .unwrap();
    assert_eq!(stopped.status, ToolStatus::Success);
    assert_eq!(stopped.data["status"], json!("stopped"));

    // Give cancelled background task a chance to flush stale completion.
    tokio::time::sleep(Duration::from_millis(30)).await;

    let resumed = run_tool
        .execute(
            json!({
                "run_id": run_id,
                "prompt":"resume",
                "background": false
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ToolStatus::Success);
    assert_eq!(resumed.data["status"], json!("completed"));
}

#[tokio::test]
async fn manager_stop_tree_stops_descendants() {
    let manager = AgentRunManager::new();
    manager
        .put_running(
            "parent-run",
            "owner-thread".to_string(),
            "agent-a".to_string(),
            None,
            Thread::new("parent-run-thread"),
            None,
        )
        .await;
    manager
        .put_running(
            "child-run",
            "owner-thread".to_string(),
            "agent-a".to_string(),
            Some("parent-run".to_string()),
            Thread::new("child-run-thread"),
            None,
        )
        .await;
    manager
        .put_running(
            "grandchild-run",
            "owner-thread".to_string(),
            "agent-a".to_string(),
            Some("child-run".to_string()),
            Thread::new("grandchild-run-thread"),
            None,
        )
        .await;
    manager
        .put_running(
            "other-owner-run",
            "other-owner".to_string(),
            "agent-b".to_string(),
            Some("parent-run".to_string()),
            Thread::new("other-owner-thread"),
            None,
        )
        .await;

    let stopped = manager
        .stop_owned_tree("owner-thread", "parent-run")
        .await
        .unwrap();

    assert_eq!(stopped.len(), 3);

    let parent = manager
        .get_owned_summary("owner-thread", "parent-run")
        .await
        .expect("parent run should exist");
    assert_eq!(parent.status, AgentRunStatus::Stopped);

    let child = manager
        .get_owned_summary("owner-thread", "child-run")
        .await
        .expect("child run should exist");
    assert_eq!(child.status, AgentRunStatus::Stopped);

    let grandchild = manager
        .get_owned_summary("owner-thread", "grandchild-run")
        .await
        .expect("grandchild run should exist");
    assert_eq!(grandchild.status, AgentRunStatus::Stopped);

    let denied = manager
        .stop_owned_tree("owner-thread", "other-owner-run")
        .await;
    assert!(denied.is_err());
}

#[tokio::test]
async fn agent_run_tool_persists_run_state_patch() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                .with_plugin(Arc::new(SlowSkipPlugin)),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));

    let doc = json!({});
    let rt = caller_runtime();
    let ctx = crate::contracts::context::Context::new(&doc, "call-run", "tool:agent_run").with_scope(Some(&rt));
    let started = run_tool
        .execute(
            json!({
                "agent_id":"worker",
                "prompt":"start",
                "background": true
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(started.status, ToolStatus::Success);
    let run_id = started.data["run_id"]
        .as_str()
        .expect("run_id should exist")
        .to_string();

    let patch = ctx.take_patch();
    assert!(
        !patch.patch().is_empty(),
        "expected tool to persist run snapshot into state"
    );
    let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["agent"]["agent_runs"][&run_id]["status"],
        json!("running")
    );
}

#[tokio::test]
async fn agent_run_tool_binds_runtime_run_id_and_parent_lineage() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                .with_plugin(Arc::new(SlowSkipPlugin)),
        )
        .build()
        .unwrap();
    let manager = Arc::new(AgentRunManager::new());
    let run_tool = AgentRunTool::new(os, manager.clone());

    let doc = json!({});
    let rt = caller_runtime_with_state_and_run(json!({"forked": true}), "parent-run-42");
    let ctx = crate::contracts::context::Context::new(&doc, "call-run", "tool:agent_run").with_scope(Some(&rt));
    let started = run_tool
        .execute(
            json!({
                "agent_id":"worker",
                "prompt":"start",
                "background": true
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(started.status, ToolStatus::Success);
    let run_id = started.data["run_id"]
        .as_str()
        .expect("run_id should exist")
        .to_string();

    let child_thread = manager
        .owned_record("owner-thread", &run_id)
        .await
        .expect("child thread should be tracked");
    assert_eq!(
        child_thread
            .runtime
            .value(RUNTIME_RUN_ID_KEY)
            .and_then(|v| v.as_str()),
        Some(run_id.as_str())
    );
    assert_eq!(
        child_thread
            .runtime
            .value(RUNTIME_PARENT_RUN_ID_KEY)
            .and_then(|v| v.as_str()),
        Some("parent-run-42")
    );

    let patch = ctx.take_patch();
    let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["agent"]["agent_runs"][&run_id]["parent_run_id"],
        json!("parent-run-42")
    );
}

#[tokio::test]
async fn agent_run_tool_resumes_from_persisted_state_without_live_record() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                .with_plugin(Arc::new(SlowSkipPlugin)),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));

    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let doc = json!({
        "agent": {
            "agent_runs": {
                "run-1": {
                    "run_id": "run-1",
                    "target_agent_id": "worker",
                    "status": "stopped",
                    "thread": serde_json::to_value(&child_thread).unwrap()
                }
            }
        }
    });
    let rt = caller_runtime_with_state(doc.clone());
    let ctx = crate::contracts::context::Context::new(&doc, "call-run", "tool:agent_run").with_scope(Some(&rt));
    let resumed = run_tool
        .execute(
            json!({
                "run_id":"run-1",
                "prompt":"resume",
                "background": false
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ToolStatus::Success);
    assert_eq!(resumed.data["status"], json!("completed"));
}

#[tokio::test]
async fn agent_run_tool_resume_updates_parent_run_lineage() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                .with_plugin(Arc::new(SlowSkipPlugin)),
        )
        .build()
        .unwrap();
    let manager = Arc::new(AgentRunManager::new());
    let run_tool = AgentRunTool::new(os, manager.clone());

    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let doc = json!({
        "agent": {
            "agent_runs": {
                "run-1": {
                    "run_id": "run-1",
                    "parent_run_id": "old-parent",
                    "target_agent_id": "worker",
                    "status": "stopped",
                    "thread": serde_json::to_value(&child_thread).unwrap()
                }
            }
        }
    });
    let rt = caller_runtime_with_state_and_run(doc.clone(), "new-parent-run");
    let ctx = crate::contracts::context::Context::new(&doc, "call-run", "tool:agent_run").with_scope(Some(&rt));
    let resumed = run_tool
        .execute(
            json!({
                "run_id":"run-1",
                "prompt":"resume",
                "background": false
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ToolStatus::Success);

    let child_thread = manager
        .owned_record("owner-thread", "run-1")
        .await
        .expect("resumed run should be tracked");
    assert_eq!(
        child_thread
            .runtime
            .value(RUNTIME_RUN_ID_KEY)
            .and_then(|v| v.as_str()),
        Some("run-1")
    );
    assert_eq!(
        child_thread
            .runtime
            .value(RUNTIME_PARENT_RUN_ID_KEY)
            .and_then(|v| v.as_str()),
        Some("new-parent-run")
    );

    let patch = ctx.take_patch();
    let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["agent"]["agent_runs"]["run-1"]["parent_run_id"],
        json!("new-parent-run")
    );
}

#[tokio::test]
async fn agent_run_tool_marks_orphan_running_as_stopped_before_resume() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                .with_plugin(Arc::new(SlowSkipPlugin)),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));

    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let doc = json!({
        "agent": {
            "agent_runs": {
                "run-1": {
                    "run_id": "run-1",
                    "target_agent_id": "worker",
                    "status": "running",
                    "thread": serde_json::to_value(&child_thread).unwrap()
                }
            }
        }
    });
    let rt = caller_runtime_with_state(doc.clone());
    let ctx = crate::contracts::context::Context::new(&doc, "call-run", "tool:agent_run").with_scope(Some(&rt));
    let summary = run_tool
        .execute(
            json!({
                "run_id":"run-1",
                "background": false
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(summary.status, ToolStatus::Success);
    assert_eq!(summary.data["status"], json!("stopped"));
}

#[tokio::test]
async fn agent_stop_tool_stops_descendant_runs() {
    let manager = Arc::new(AgentRunManager::new());
    let stop_tool = AgentStopTool::new(manager.clone());
    let os_thread = Thread::new("owner-thread");

    let parent_thread = Thread::new("parent-s");
    let child_thread = Thread::new("child-s");
    let grandchild_thread = Thread::new("grandchild-s");
    let parent_run_id = "run-parent";
    let child_run_id = "run-child";
    let grandchild_run_id = "run-grandchild";

    manager
        .put_running(
            parent_run_id,
            os_thread.id.clone(),
            "worker".to_string(),
            None,
            parent_thread.clone(),
            None,
        )
        .await;
    manager
        .put_running(
            child_run_id,
            os_thread.id.clone(),
            "worker".to_string(),
            Some(parent_run_id.to_string()),
            child_thread.clone(),
            None,
        )
        .await;
    manager
        .put_running(
            grandchild_run_id,
            os_thread.id.clone(),
            "worker".to_string(),
            Some(child_run_id.to_string()),
            grandchild_thread.clone(),
            None,
        )
        .await;

    let doc = json!({
        "agent": {
            "agent_runs": {
                parent_run_id: {
                    "run_id": parent_run_id,
                    "target_agent_id": "worker",
                    "status": "running",
                    "thread": serde_json::to_value(parent_thread).unwrap()
                },
                child_run_id: {
                    "run_id": child_run_id,
                    "parent_run_id": parent_run_id,
                    "target_agent_id": "worker",
                    "status": "running",
                    "thread": serde_json::to_value(child_thread).unwrap()
                },
                grandchild_run_id: {
                    "run_id": grandchild_run_id,
                    "parent_run_id": child_run_id,
                    "target_agent_id": "worker",
                    "status": "running",
                    "thread": serde_json::to_value(grandchild_thread).unwrap()
                }
            }
        }
    });

    let mut rt = carve_state::ScopeState::new();
    rt.set(TOOL_RUNTIME_CALLER_THREAD_ID_KEY, os_thread.id.clone())
        .unwrap();
    let ctx =
        crate::contracts::context::Context::new(&doc, "call-stop", "tool:agent_stop").with_scope(Some(&rt));
    let result = stop_tool
        .execute(json!({ "run_id": parent_run_id }), &ctx)
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);
    assert_eq!(result.data["status"], json!("stopped"));

    let parent = manager
        .get_owned_summary(&os_thread.id, parent_run_id)
        .await
        .expect("parent run should exist");
    assert_eq!(parent.status, AgentRunStatus::Stopped);
    let child = manager
        .get_owned_summary(&os_thread.id, child_run_id)
        .await
        .expect("child run should exist");
    assert_eq!(child.status, AgentRunStatus::Stopped);
    let grandchild = manager
        .get_owned_summary(&os_thread.id, grandchild_run_id)
        .await
        .expect("grandchild run should exist");
    assert_eq!(grandchild.status, AgentRunStatus::Stopped);

    let patch = ctx.take_patch();
    let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["agent"]["agent_runs"][parent_run_id]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["agent"]["agent_runs"][child_run_id]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["agent"]["agent_runs"][grandchild_run_id]["status"],
        json!("stopped")
    );
}

#[tokio::test]
async fn recovery_plugin_reconciles_orphan_running_and_requests_confirmation() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let ctx = Context::new(&doc, "test", "test");
    let mut step = StepContext::new(&thread, vec![]);
    plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;
    assert!(
        !step.pending_patches.is_empty(),
        "expected reconciliation + pending patches for orphan running entry"
    );
    assert!(!step.skip_inference);

    let updated = thread
        .clone()
        .with_patches(step.pending_patches.clone())
        .rebuild_state()
        .unwrap();
    assert_eq!(
        updated["agent"]["agent_runs"]["run-1"]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["agent"]["pending_interaction"]["action"],
        json!(AGENT_RECOVERY_INTERACTION_ACTION)
    );
    assert_eq!(
        updated["agent"]["pending_interaction"]["parameters"]["run_id"],
        json!("run-1")
    );

    let updated_thread = thread.clone().with_patches(step.pending_patches);
    let mut before = StepContext::new(&updated_thread, vec![]);
    plugin
        .on_phase(Phase::BeforeInference, &mut before, &ctx)
        .await;
    assert!(
        before.skip_inference,
        "recovery confirmation should pause inference"
    );
}

#[tokio::test]
async fn recovery_plugin_does_not_override_existing_pending_interaction() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "agent": {
                "pending_interaction": {
                    "id": "existing_1",
                    "action": "confirm",
                },
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        }),
    );

    let doc = thread.rebuild_state().unwrap();
    let ctx = Context::new(&doc, "test", "test");
    let mut step = StepContext::new(&thread, vec![]);
    plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;
    assert!(
        !step.skip_inference,
        "existing pending interaction should not be replaced"
    );

    let updated = thread
        .clone()
        .with_patches(step.pending_patches)
        .rebuild_state()
        .unwrap();
    assert_eq!(
        updated["agent"]["pending_interaction"]["id"],
        json!("existing_1")
    );
}

#[tokio::test]
async fn recovery_plugin_auto_approve_when_permission_allow() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {
                    "recover_agent_run": "allow"
                }
            },
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let ctx = Context::new(&doc, "test", "test");
    let mut step = StepContext::new(&thread, vec![]);
    plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;

    let updated = thread
        .clone()
        .with_patches(step.pending_patches)
        .rebuild_state()
        .unwrap();
    let replay_calls: Vec<ToolCall> = updated["agent"]
        .get("replay_tool_calls")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    assert_eq!(replay_calls.len(), 1);
    assert_eq!(replay_calls[0].name, "agent_run");
    assert_eq!(replay_calls[0].arguments["run_id"], "run-1");
    assert_eq!(
        updated["agent"]["agent_runs"]["run-1"]["status"],
        json!("stopped")
    );
    assert!(
        updated["agent"].get("pending_interaction").is_none()
            || updated["agent"]["pending_interaction"].is_null()
    );
}

#[tokio::test]
async fn recovery_plugin_auto_deny_when_permission_deny() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {
                    "recover_agent_run": "deny"
                }
            },
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let ctx = Context::new(&doc, "test", "test");
    let mut step = StepContext::new(&thread, vec![]);
    plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;

    let updated = thread
        .clone()
        .with_patches(step.pending_patches)
        .rebuild_state()
        .unwrap();
    let replay_calls: Vec<ToolCall> = updated["agent"]
        .get("replay_tool_calls")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    assert!(replay_calls.is_empty());
    assert_eq!(
        updated["agent"]["agent_runs"]["run-1"]["status"],
        json!("stopped")
    );
    assert!(
        updated["agent"].get("pending_interaction").is_none()
            || updated["agent"]["pending_interaction"].is_null()
    );
}

#[tokio::test]
async fn recovery_plugin_auto_approve_from_default_behavior_allow() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            },
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let ctx = Context::new(&doc, "test", "test");
    let mut step = StepContext::new(&thread, vec![]);
    plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;

    let updated = thread
        .clone()
        .with_patches(step.pending_patches)
        .rebuild_state()
        .unwrap();
    let replay_calls: Vec<ToolCall> = updated["agent"]
        .get("replay_tool_calls")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    assert_eq!(replay_calls.len(), 1);
    assert_eq!(replay_calls[0].name, "agent_run");
    assert_eq!(replay_calls[0].arguments["run_id"], "run-1");
    assert!(
        updated["agent"].get("pending_interaction").is_none()
            || updated["agent"]["pending_interaction"].is_null()
    );
}

#[tokio::test]
async fn recovery_plugin_auto_deny_from_default_behavior_deny() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            },
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let ctx = Context::new(&doc, "test", "test");
    let mut step = StepContext::new(&thread, vec![]);
    plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;

    let updated = thread
        .clone()
        .with_patches(step.pending_patches)
        .rebuild_state()
        .unwrap();
    let replay_calls: Vec<ToolCall> = updated["agent"]
        .get("replay_tool_calls")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    assert!(
        replay_calls.is_empty(),
        "deny should not schedule recovery replay"
    );
    assert!(
        updated["agent"].get("pending_interaction").is_none()
            || updated["agent"]["pending_interaction"].is_null()
    );
}

#[tokio::test]
async fn recovery_plugin_tool_rule_overrides_default_behavior() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
    let child_thread = crate::contracts::conversation::Thread::new("child-run")
        .with_message(crate::contracts::conversation::Message::user("seed"));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {
                    "recover_agent_run": "ask"
                }
            },
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let ctx = Context::new(&doc, "test", "test");
    let mut step = StepContext::new(&thread, vec![]);
    plugin.on_phase(Phase::RunStart, &mut step, &ctx).await;

    let updated = thread
        .clone()
        .with_patches(step.pending_patches)
        .rebuild_state()
        .unwrap();
    let replay_calls: Vec<ToolCall> = updated["agent"]
        .get("replay_tool_calls")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    assert!(
        replay_calls.is_empty(),
        "tool-level ask should override default allow"
    );
    assert_eq!(
        updated["agent"]["pending_interaction"]["action"],
        json!(AGENT_RECOVERY_INTERACTION_ACTION)
    );
}
