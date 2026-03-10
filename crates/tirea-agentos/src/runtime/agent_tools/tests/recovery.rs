use super::*;
use crate::loop_runtime::loop_runner::RunCancellationToken;
use crate::runtime::background_tasks::TaskResult;

// Helper: build persisted-state JSON for a single running agent_run task.
fn running_task_json(run_id: &str, agent_id: &str) -> serde_json::Value {
    json!({
        "task_type": "agent_run",
        "description": format!("agent:{agent_id}"),
        "status": "running",
        "created_at_ms": 0,
        "metadata": {
            "thread_id": format!("sub-agent-{run_id}"),
            "agent_id": agent_id
        }
    })
}

// ── AgentRecoveryPlugin tests ────────────────────────────────────────────────

#[tokio::test]
async fn recovery_plugin_detects_orphan_and_records_confirmation() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
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
        updated["background_tasks"]["tasks"]["run-1"]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["suspension"]
            ["action"],
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
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
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
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
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
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {
                    "recover_agent_run": "allow"
                }
            },
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
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
        updated["background_tasks"]["tasks"]["run-1"]["status"],
        json!("stopped")
    );
}

#[cfg(feature = "permission")]
#[tokio::test]
async fn recovery_plugin_auto_deny_when_permission_deny() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "ask",
                "tools": {
                    "recover_agent_run": "deny"
                }
            },
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
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
        updated["background_tasks"]["tasks"]["run-1"]["status"],
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
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {}
            },
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
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

#[cfg(feature = "permission")]
#[tokio::test]
async fn recovery_plugin_auto_deny_from_default_behavior_deny() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {}
            },
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
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

#[cfg(feature = "permission")]
#[tokio::test]
async fn recovery_plugin_tool_rule_overrides_default_behavior() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "allow",
                "tools": {
                    "recover_agent_run": "ask"
                }
            },
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
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
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["suspension"]
            ["action"],
        json!(AGENT_RECOVERY_INTERACTION_ACTION)
    );
}

// ── Recovery plugin: multiple orphans ────────────────────────────────────────

#[tokio::test]
async fn recovery_plugin_detects_multiple_orphans_creates_one_suspension() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker-a"),
                    "run-2": running_task_json("run-2", "worker-b"),
                    "run-3": running_task_json("run-3", "worker-c")
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();

    // All 3 should be marked stopped.
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-1"]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-2"]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-3"]["status"],
        json!("stopped")
    );

    // Only one recovery suspension should be created (for the first orphan).
    let scope = &updated["__tool_call_scope"];
    let suspended_count = scope
        .as_object()
        .map(|obj| {
            obj.values()
                .filter(|v| v.get("suspended_call").is_some())
                .count()
        })
        .unwrap_or(0);
    assert_eq!(
        suspended_count, 1,
        "only one recovery suspension should be created"
    );
}

// ── Recovery plugin: mix of orphans and live handles ─────────────────────────

#[tokio::test]
async fn recovery_plugin_only_marks_orphans_when_some_have_live_handles() {
    let bg_mgr = Arc::new(BackgroundTaskManager::new());
    // run-1 has a live handle in BackgroundTaskManager.
    let token = RunCancellationToken::new();
    bg_mgr
        .spawn_with_id(
            "run-1".to_string(),
            "owner-1",
            "agent_run",
            "agent:worker-a",
            token.clone(),
            None,
            json!({"agent_id": "worker-a", "thread_id": "sub-agent-run-1"}),
            |cancel| async move {
                cancel.cancelled().await;
                TaskResult::Cancelled
            },
        )
        .await;

    let plugin = AgentRecoveryPlugin::new(bg_mgr);
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker-a"),
                    "run-2": running_task_json("run-2", "worker-b")
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();

    // run-1 has live handle -> not marked stopped.
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-1"]["status"],
        json!("running")
    );

    // run-2 is orphaned -> marked stopped.
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-2"]["status"],
        json!("stopped")
    );
}

// ── Recovery plugin: no orphans when all have live handles ───────────────────

#[tokio::test]
async fn recovery_plugin_no_action_when_all_running_have_live_handles() {
    let bg_mgr = Arc::new(BackgroundTaskManager::new());
    let token1 = RunCancellationToken::new();
    bg_mgr
        .spawn_with_id(
            "run-1".to_string(),
            "owner-1",
            "agent_run",
            "agent:worker-a",
            token1.clone(),
            None,
            json!({"agent_id": "worker-a", "thread_id": "sub-agent-run-1"}),
            |cancel| async move {
                cancel.cancelled().await;
                TaskResult::Cancelled
            },
        )
        .await;
    let token2 = RunCancellationToken::new();
    bg_mgr
        .spawn_with_id(
            "run-2".to_string(),
            "owner-1",
            "agent_run",
            "agent:worker-b",
            token2.clone(),
            None,
            json!({"agent_id": "worker-b", "thread_id": "sub-agent-run-2"}),
            |cancel| async move {
                cancel.cancelled().await;
                TaskResult::Cancelled
            },
        )
        .await;

    let plugin = AgentRecoveryPlugin::new(bg_mgr);
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker-a"),
                    "run-2": running_task_json("run-2", "worker-b")
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();

    // Both still running (no orphan detection).
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-1"]["status"],
        json!("running")
    );
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-2"]["status"],
        json!("running")
    );

    // No suspended recovery interaction.
    let has_suspended = updated
        .get("__tool_call_scope")
        .and_then(|scope| scope.as_object())
        .map(|obj| obj.values().any(|v| v.get("suspended_call").is_some()))
        .unwrap_or(false);
    assert!(
        !has_suspended,
        "no recovery suspension should be created when all have live handles"
    );
}

// ── Recovery plugin: mixed statuses only Running without handle is orphan ────

#[tokio::test]
async fn recovery_plugin_ignores_completed_stopped_failed_in_persisted_state() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "background_tasks": {
                "tasks": {
                    "run-completed": {
                        "task_type": "agent_run",
                        "description": "agent:worker",
                        "status": "completed",
                        "created_at_ms": 0,
                        "metadata": {
                            "thread_id": "sub-agent-run-completed",
                            "agent_id": "worker"
                        }
                    },
                    "run-failed": {
                        "task_type": "agent_run",
                        "description": "agent:worker",
                        "status": "failed",
                        "created_at_ms": 0,
                        "error": "oops",
                        "metadata": {
                            "thread_id": "sub-agent-run-failed",
                            "agent_id": "worker"
                        }
                    },
                    "run-stopped": {
                        "task_type": "agent_run",
                        "description": "agent:worker",
                        "status": "stopped",
                        "created_at_ms": 0,
                        "metadata": {
                            "thread_id": "sub-agent-run-stopped",
                            "agent_id": "worker"
                        }
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

    // None of them should change status.
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-completed"]["status"],
        json!("completed")
    );
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-failed"]["status"],
        json!("failed")
    );
    assert_eq!(
        updated["background_tasks"]["tasks"]["run-stopped"]["status"],
        json!("stopped")
    );

    // No suspended recovery interaction.
    let has_suspended = updated
        .get("__tool_call_scope")
        .and_then(|scope| scope.as_object())
        .map(|obj| obj.values().any(|v| v.get("suspended_call").is_some()))
        .unwrap_or(false);
    assert!(!has_suspended);
}

// ── Permission fallback test ─────────────────────────────────────────────────

#[cfg(not(feature = "permission"))]
#[tokio::test]
async fn recovery_plugin_fallback_always_approves_despite_deny_state() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(BackgroundTaskManager::new()));
    // State declares "deny" for recover_agent_run, but without the permission
    // feature the fallback always returns Allow.
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "permissions": {
                "default_behavior": "deny",
                "tools": {
                    "recover_agent_run": "deny"
                }
            },
            "background_tasks": {
                "tasks": {
                    "run-1": running_task_json("run-1", "worker")
                }
            }
        }),
    );
    let doc = thread.rebuild_state().unwrap();
    let fixture = TestFixture::new_with_state(doc);
    let mut step = fixture.step(vec![]);
    plugin.run_phase(Phase::RunStart, &mut step).await;

    let updated = fixture.updated_state();
    // Without permission feature, fallback always returns Allow -- so recovery
    // should auto-approve even though state says "deny".
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["tool_call_state"]["status"],
        json!("resuming"),
        "fallback should auto-approve when permission feature is off"
    );
    assert_eq!(
        updated["__tool_call_scope"]["agent_recovery_run-1"]["tool_call_state"]["resume"]["action"],
        json!("resume"),
    );
}
