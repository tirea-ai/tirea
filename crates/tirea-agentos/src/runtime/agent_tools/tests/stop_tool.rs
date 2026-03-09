use super::*;

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

// ── AgentStopTool edge cases ─────────────────────────────────────────────────

#[tokio::test]
async fn agent_stop_tool_requires_scope_context() {
    let handles = Arc::new(SubAgentHandleTable::new());
    let stop_tool = AgentStopTool::new(handles);
    let fix = TestFixture::new();

    let result = stop_tool
        .execute(
            json!({ "run_id": "run-1" }),
            &fix.ctx_with("call-1", "tool:agent_stop"),
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
async fn agent_stop_tool_requires_run_id() {
    let handles = Arc::new(SubAgentHandleTable::new());
    let stop_tool = AgentStopTool::new(handles);
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();

    let result = stop_tool
        .execute(json!({}), &fix.ctx_with("call-1", "tool:agent_stop"))
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("missing 'run_id'"));
}

#[tokio::test]
async fn agent_stop_tool_stops_persisted_running_without_live_handle() {
    let handles = Arc::new(SubAgentHandleTable::new());
    let stop_tool = AgentStopTool::new(handles);

    let doc = json!({
        "sub_agents": {
            "runs": {
                "run-orphan": {
                    "thread_id": "sub-agent-run-orphan",
                    "agent_id": "worker",
                    "status": "running"
                }
            }
        }
    });
    let mut fix = TestFixture::new_with_state(doc.clone());
    fix.run_config = {
        let mut rt = tirea_contract::RunConfig::new();
        rt.set(TOOL_SCOPE_CALLER_THREAD_ID_KEY, "owner-thread")
            .unwrap();
        rt
    };

    let result = stop_tool
        .execute(
            json!({ "run_id": "run-orphan" }),
            &fix.ctx_with("call-stop", "tool:agent_stop"),
        )
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Success);
    assert_eq!(result.data["status"], json!("stopped"));

    // Verify the state patch marks it stopped.
    let patch = fix.ctx_with("call-stop", "tool:agent_stop").take_patch();
    let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
    assert_eq!(
        updated["sub_agents"]["runs"]["run-orphan"]["status"],
        json!("stopped")
    );
}
