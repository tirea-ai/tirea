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
    let bg_mgr = Arc::new(BackgroundTaskManager::new());
    let run_tool = AgentRunTool::new(os, bg_mgr.clone());

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

    // All should show as running in BackgroundTaskManager.
    let running = bg_mgr
        .list(
            "owner-thread",
            Some(crate::runtime::background_tasks::TaskStatus::Running),
        )
        .await;
    assert_eq!(running.len(), 3);

    // Cancel each one via BackgroundTaskManager.
    for run_id in &run_ids {
        bg_mgr.cancel("owner-thread", run_id).await.unwrap();
    }

    // Wait for background tasks to flush.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // All should show as cancelled.
    let cancelled_all = bg_mgr
        .list(
            "owner-thread",
            Some(crate::runtime::background_tasks::TaskStatus::Cancelled),
        )
        .await;
    assert_eq!(cancelled_all.len(), 3);
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
    let bg_mgr = Arc::new(BackgroundTaskManager::new());
    let run_tool = AgentRunTool::new(os, bg_mgr.clone());

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

    // All should be visible as running in BackgroundTaskManager.
    let running = bg_mgr
        .list(
            "owner-thread",
            Some(crate::runtime::background_tasks::TaskStatus::Running),
        )
        .await;
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
    let bg_mgr = Arc::new(BackgroundTaskManager::new());
    let run_tool = AgentRunTool::new(os, bg_mgr.clone());

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

    // Immediately cancel via BackgroundTaskManager (race with background execution).
    bg_mgr.cancel("owner-thread", &run_id).await.unwrap();

    // Wait for background task to flush.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify the run is cancelled (cancellation override should prevent it from flipping to completed).
    let task = bg_mgr.get("owner-thread", &run_id).await.unwrap();
    assert_eq!(
        task.status,
        crate::runtime::background_tasks::TaskStatus::Cancelled
    );
}
