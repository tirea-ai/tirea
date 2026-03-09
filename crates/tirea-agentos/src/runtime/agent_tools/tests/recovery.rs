use super::*;

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

#[cfg(feature = "permission")]
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

#[cfg(feature = "permission")]
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

#[cfg(feature = "permission")]
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
        updated["__tool_call_scope"]["agent_recovery_run-1"]["suspended_call"]["suspension"]
            ["action"],
        json!(AGENT_RECOVERY_INTERACTION_ACTION)
    );
}

// ── Recovery plugin: multiple orphans ────────────────────────────────────────

#[tokio::test]
async fn recovery_plugin_detects_multiple_orphans_creates_one_suspension() {
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker-a",
                        "status": "running"
                    },
                    "run-2": {
                        "thread_id": "sub-agent-run-2",
                        "agent_id": "worker-b",
                        "status": "running"
                    },
                    "run-3": {
                        "thread_id": "sub-agent-run-3",
                        "agent_id": "worker-c",
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

    // All 3 should be marked stopped.
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-2"]["status"],
        json!("stopped")
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-3"]["status"],
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
    let handles = Arc::new(SubAgentHandleTable::new());
    // run-1 has a live handle.
    handles
        .put_running(
            "run-1",
            "owner-1".to_string(),
            "sub-agent-run-1".to_string(),
            "worker-a".to_string(),
            None,
            None,
        )
        .await;

    let plugin = AgentRecoveryPlugin::new(handles);
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker-a",
                        "status": "running"
                    },
                    "run-2": {
                        "thread_id": "sub-agent-run-2",
                        "agent_id": "worker-b",
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

    // run-1 has live handle -> not marked stopped.
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["status"],
        json!("running")
    );

    // run-2 is orphaned -> marked stopped.
    assert_eq!(
        updated["sub_agents"]["runs"]["run-2"]["status"],
        json!("stopped")
    );
}

// ── Recovery plugin: no orphans when all have live handles ───────────────────

#[tokio::test]
async fn recovery_plugin_no_action_when_all_running_have_live_handles() {
    let handles = Arc::new(SubAgentHandleTable::new());
    handles
        .put_running(
            "run-1",
            "owner-1".to_string(),
            "sub-agent-run-1".to_string(),
            "worker-a".to_string(),
            None,
            None,
        )
        .await;
    handles
        .put_running(
            "run-2",
            "owner-1".to_string(),
            "sub-agent-run-2".to_string(),
            "worker-b".to_string(),
            None,
            None,
        )
        .await;

    let plugin = AgentRecoveryPlugin::new(handles);
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "sub_agents": {
                "runs": {
                    "run-1": {
                        "thread_id": "sub-agent-run-1",
                        "agent_id": "worker-a",
                        "status": "running"
                    },
                    "run-2": {
                        "thread_id": "sub-agent-run-2",
                        "agent_id": "worker-b",
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

    // Both still running (no orphan detection).
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["status"],
        json!("running")
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-2"]["status"],
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
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
    let thread = Thread::with_initial_state(
        "owner-1",
        json!({
            "sub_agents": {
                "runs": {
                    "run-completed": {
                        "thread_id": "sub-agent-run-completed",
                        "agent_id": "worker",
                        "status": "completed"
                    },
                    "run-failed": {
                        "thread_id": "sub-agent-run-failed",
                        "agent_id": "worker",
                        "status": "failed",
                        "error": "oops"
                    },
                    "run-stopped": {
                        "thread_id": "sub-agent-run-stopped",
                        "agent_id": "worker",
                        "status": "stopped"
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
        updated["sub_agents"]["runs"]["run-completed"]["status"],
        json!("completed")
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-failed"]["status"],
        json!("failed")
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-stopped"]["status"],
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
    let plugin = AgentRecoveryPlugin::new(Arc::new(SubAgentHandleTable::new()));
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
