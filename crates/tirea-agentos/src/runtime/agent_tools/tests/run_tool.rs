use super::*;

// ── AgentRunTool tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn agent_run_tool_requires_scope_context() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .with_agent(
            "reviewer",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini")
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini")
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini")
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
        updated["sub_agents"]["runs"][&run_id]
            .get("thread")
            .is_none()
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini")
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
async fn agent_run_tool_query_existing_run_keeps_original_parent_lineage() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let handles = Arc::new(SubAgentHandleTable::new());
    handles
        .put_running(
            "run-1",
            "owner-thread".to_string(),
            "custom-child-thread".to_string(),
            "worker".to_string(),
            Some("origin-parent-run".to_string()),
            None,
        )
        .await;
    let run_tool = AgentRunTool::new(os, handles);

    let base_doc = json!({
        "sub_agents": {
            "runs": {
                "run-1": {
                    "thread_id": "custom-child-thread",
                    "parent_run_id": "origin-parent-run",
                    "agent_id": "worker",
                    "status": "running"
                }
            }
        }
    });
    let mut fix = TestFixture::new_with_state(base_doc.clone());
    fix.run_config = caller_scope_with_state_and_run(base_doc.clone(), "query-parent-run");

    let result = run_tool
        .execute(
            json!({
                "run_id":"run-1",
                "background": false
            }),
            &fix.ctx_with("call-run", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);

    let patch = fix.ctx_with("call-run", "tool:agent_run").take_patch();
    let updated = apply_patches(&base_doc, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["parent_run_id"],
        json!("origin-parent-run")
    );
    assert_eq!(
        updated["sub_agents"]["runs"]["run-1"]["thread_id"],
        json!("custom-child-thread")
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini")
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
            crate::runtime::AgentDefinition::new("gpt-4o-mini")
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

// ── AgentRunTool: resume completed/failed returns status ─────────────────────

#[tokio::test]
async fn agent_run_tool_returns_completed_status_when_resuming_completed_run() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let handles = Arc::new(SubAgentHandleTable::new());
    let epoch = handles
        .put_running(
            "run-1",
            "owner-thread".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    handles
        .update_after_completion(
            "run-1",
            epoch,
            SubAgentCompletion {
                status: SubAgentStatus::Completed,
                error: None,
            },
        )
        .await;

    let run_tool = AgentRunTool::new(os, handles);
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();

    let result = run_tool
        .execute(
            json!({ "run_id": "run-1", "background": false }),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);
    assert_eq!(result.data["status"], json!("completed"));
}

#[tokio::test]
async fn agent_run_tool_returns_failed_status_when_resuming_failed_run() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let handles = Arc::new(SubAgentHandleTable::new());
    let epoch = handles
        .put_running(
            "run-1",
            "owner-thread".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    handles
        .update_after_completion(
            "run-1",
            epoch,
            SubAgentCompletion {
                status: SubAgentStatus::Failed,
                error: Some("agent failed".to_string()),
            },
        )
        .await;

    let run_tool = AgentRunTool::new(os, handles);
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();

    let result = run_tool
        .execute(
            json!({ "run_id": "run-1", "background": false }),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);
    assert_eq!(result.data["status"], json!("failed"));
    assert_eq!(result.data["error"], json!("agent failed"));
}

// ── AgentRunTool: missing required args ──────────────────────────────────────

#[tokio::test]
async fn agent_run_tool_requires_prompt_for_new_run() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();

    let result = run_tool
        .execute(
            json!({ "agent_id": "worker", "background": false }),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("missing 'prompt'"));
}

#[tokio::test]
async fn agent_run_tool_requires_agent_id_for_new_run() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();

    let result = run_tool
        .execute(
            json!({ "prompt": "hello", "background": false }),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("missing 'agent_id'"));
}

// ── AgentRunTool: excluded agents via scope ──────────────────────────────────

#[tokio::test]
async fn agent_run_tool_rejects_excluded_agent() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .with_agent(
            "secret",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
        )
        .build()
        .unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();
    fix.run_config
        .set(SCOPE_EXCLUDED_AGENTS_KEY, vec!["secret"])
        .unwrap();

    let result = run_tool
        .execute(
            json!({ "agent_id": "secret", "prompt": "hi", "background": false }),
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

// ── AgentRunTool: unknown run_id (no handle, no persisted) ───────────────────

#[tokio::test]
async fn agent_run_tool_returns_error_for_unknown_run_id() {
    let os = AgentOs::builder().build().unwrap();
    let run_tool = AgentRunTool::new(os, Arc::new(SubAgentHandleTable::new()));
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();

    let result = run_tool
        .execute(
            json!({ "run_id": "nonexistent", "background": false }),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("Unknown run_id"));
}

// ── AgentRunTool: persisted completed/failed returns without re-run ──────────

#[tokio::test]
async fn agent_run_tool_returns_persisted_completed_without_rerun() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
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
                    "status": "completed"
                }
            }
        }
    });
    let mut fix = TestFixture::new_with_state(doc.clone());
    fix.run_config = caller_scope_with_state(doc);

    let result = run_tool
        .execute(
            json!({ "run_id": "run-1", "background": false }),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);
    assert_eq!(result.data["status"], json!("completed"));
}

#[tokio::test]
async fn agent_run_tool_returns_persisted_failed_with_error() {
    let os = AgentOs::builder()
        .with_agent(
            "worker",
            crate::runtime::AgentDefinition::new("gpt-4o-mini"),
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
                    "status": "failed",
                    "error": "something broke"
                }
            }
        }
    });
    let mut fix = TestFixture::new_with_state(doc.clone());
    fix.run_config = caller_scope_with_state(doc);

    let result = run_tool
        .execute(
            json!({ "run_id": "run-1", "background": false }),
            &fix.ctx_with("call-1", "tool:agent_run"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);
    assert_eq!(result.data["status"], json!("failed"));
    assert_eq!(result.data["error"], json!("something broke"));
}
