use super::*;

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
            crate::runtime::AgentDefinition::new("gpt-4o-mini")
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
        run_ids.push(started.data["run_id"].as_str().unwrap().to_string());
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

// ── Status lifecycle tests ───────────────────────────────────────────────────

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
    handles.stop_owned_tree("owner", "run-1").await.unwrap();

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

// ── Parallel tool-level operations ───────────────────────────────────────────

#[tokio::test]
async fn parallel_background_launches_produce_unique_run_ids() {
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

    let mut run_ids = Vec::new();
    for i in 0..5 {
        let mut fix = TestFixture::new();
        fix.run_config = caller_scope();
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
        run_ids.push(started.data["run_id"].as_str().unwrap().to_string());
    }

    // All run_ids should be unique.
    let unique: std::collections::HashSet<&str> = run_ids.iter().map(|s| s.as_str()).collect();
    assert_eq!(unique.len(), 5, "all run_ids should be unique");

    // All should be visible as running.
    let running = handles.running_or_stopped_for_owner("owner-thread").await;
    assert_eq!(running.len(), 5);
}

#[tokio::test]
async fn parallel_launch_and_immediate_stop() {
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
    let stop_tool = AgentStopTool::new(handles.clone());

    // Launch background run.
    let mut fix = TestFixture::new();
    fix.run_config = caller_scope();
    let started = run_tool
        .execute(
            json!({
                "agent_id": "worker",
                "prompt": "fast task",
                "background": true
            }),
            &fix.ctx_with("call-launch", "tool:agent_run"),
        )
        .await
        .unwrap();
    let run_id = started.data["run_id"].as_str().unwrap().to_string();

    // Immediately stop (race with background execution).
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

    // Wait for background task to flush.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify the run is stopped (cancellation override should prevent it from flipping to completed).
    let summary = handles
        .get_owned_summary("owner-thread", &run_id)
        .await
        .unwrap();
    assert_eq!(summary.status, SubAgentStatus::Stopped);
}
