use super::*;

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
async fn handle_table_remove_if_epoch_only_removes_matching_generation() {
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
    assert_eq!(epoch1, 1);
    assert_eq!(epoch2, 2);

    assert!(!handles.remove_if_epoch("run-1", epoch1).await);
    assert!(handles.contains("run-1").await);
    assert!(handles.remove_if_epoch("run-1", epoch2).await);
    assert!(!handles.contains("run-1").await);
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

// ── Ownership isolation tests ────────────────────────────────────────────────

#[tokio::test]
async fn handle_table_different_owners_cannot_see_each_others_runs() {
    let handles = SubAgentHandleTable::new();
    handles
        .put_running(
            "run-a",
            "owner-1".to_string(),
            "sub-agent-run-a".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    handles
        .put_running(
            "run-b",
            "owner-2".to_string(),
            "sub-agent-run-b".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;

    // Owner-1 can see run-a but not run-b.
    assert!(handles
        .get_owned_summary("owner-1", "run-a")
        .await
        .is_some());
    assert!(handles
        .get_owned_summary("owner-1", "run-b")
        .await
        .is_none());

    // Owner-2 can see run-b but not run-a.
    assert!(handles
        .get_owned_summary("owner-2", "run-b")
        .await
        .is_some());
    assert!(handles
        .get_owned_summary("owner-2", "run-a")
        .await
        .is_none());

    // running_or_stopped_for_owner returns only own runs.
    let owner1_runs = handles.running_or_stopped_for_owner("owner-1").await;
    assert_eq!(owner1_runs.len(), 1);
    assert_eq!(owner1_runs[0].run_id, "run-a");

    let owner2_runs = handles.running_or_stopped_for_owner("owner-2").await;
    assert_eq!(owner2_runs.len(), 1);
    assert_eq!(owner2_runs[0].run_id, "run-b");
}

#[tokio::test]
async fn handle_table_cross_owner_stop_is_denied() {
    let handles = SubAgentHandleTable::new();
    handles
        .put_running(
            "run-a",
            "owner-1".to_string(),
            "sub-agent-run-a".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;

    // Owner-2 cannot stop owner-1's run.
    let err = handles.stop_owned_tree("owner-2", "run-a").await;
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("Unknown run_id"));

    // Verify run is still running.
    let summary = handles.get_owned_summary("owner-1", "run-a").await.unwrap();
    assert_eq!(summary.status, SubAgentStatus::Running);
}

#[tokio::test]
async fn handle_for_resume_rejects_wrong_owner() {
    let handles = SubAgentHandleTable::new();
    handles
        .put_running(
            "run-1",
            "owner-1".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;

    let err = handles.handle_for_resume("owner-2", "run-1").await;
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("Unknown run_id"));
}

// ── Handle table status filter tests ─────────────────────────────────────────

#[tokio::test]
async fn running_or_stopped_for_owner_excludes_completed_and_failed() {
    let handles = SubAgentHandleTable::new();

    let epoch1 = handles
        .put_running(
            "run-running",
            "owner".to_string(),
            "sub-agent-run-running".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    let epoch2 = handles
        .put_running(
            "run-completed",
            "owner".to_string(),
            "sub-agent-run-completed".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    let epoch3 = handles
        .put_running(
            "run-failed",
            "owner".to_string(),
            "sub-agent-run-failed".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    handles
        .put_running(
            "run-stopped",
            "owner".to_string(),
            "sub-agent-run-stopped".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;

    // Transition run-completed and run-failed.
    handles
        .update_after_completion(
            "run-completed",
            epoch2,
            SubAgentCompletion {
                status: SubAgentStatus::Completed,
                error: None,
            },
        )
        .await;
    handles
        .update_after_completion(
            "run-failed",
            epoch3,
            SubAgentCompletion {
                status: SubAgentStatus::Failed,
                error: Some("boom".to_string()),
            },
        )
        .await;
    handles
        .stop_owned_tree("owner", "run-stopped")
        .await
        .unwrap();

    // Verify: only Running and Stopped returned.
    let visible = handles.running_or_stopped_for_owner("owner").await;
    let visible_ids: Vec<&str> = visible.iter().map(|s| s.run_id.as_str()).collect();
    assert!(visible_ids.contains(&"run-running"));
    assert!(visible_ids.contains(&"run-stopped"));
    assert!(!visible_ids.contains(&"run-completed"));
    assert!(!visible_ids.contains(&"run-failed"));
    // Verify sort order.
    assert_eq!(visible_ids, {
        let mut sorted = visible_ids.clone();
        sorted.sort();
        sorted
    });

    // Verify epoch1 is untouched.
    let s = handles
        .get_owned_summary("owner", "run-running")
        .await
        .unwrap();
    assert_eq!(s.status, SubAgentStatus::Running);
    let _ = epoch1;
}

// ── Stop edge cases ──────────────────────────────────────────────────────────

#[tokio::test]
async fn stop_already_stopped_run_returns_error() {
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
    handles.stop_owned_tree("owner", "run-1").await.unwrap();

    // Second stop should fail.
    let result = handles.stop_owned_tree("owner", "run-1").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not running"));
}

#[tokio::test]
async fn stop_completed_run_returns_error() {
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

    let result = handles.stop_owned_tree("owner", "run-1").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not running"));
}

#[tokio::test]
async fn stop_unknown_run_returns_error() {
    let handles = SubAgentHandleTable::new();
    let result = handles.stop_owned_tree("owner", "nonexistent").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown run_id"));
}

// ── Concurrent handle table operations ───────────────────────────────────────

#[tokio::test]
async fn concurrent_put_running_same_run_id_increments_epoch() {
    let handles = Arc::new(SubAgentHandleTable::new());
    let h1 = handles.clone();
    let h2 = handles.clone();

    // Launch two registrations concurrently for the same run_id.
    let (e1, e2) = tokio::join!(
        h1.put_running(
            "run-race",
            "owner".to_string(),
            "sub-agent-run-race".to_string(),
            "worker-a".to_string(),
            None,
            None,
        ),
        h2.put_running(
            "run-race",
            "owner".to_string(),
            "sub-agent-run-race".to_string(),
            "worker-b".to_string(),
            None,
            None,
        ),
    );

    // Epochs should be different (one is 1, the other is 2, depending on ordering).
    assert_ne!(e1, e2);
    let max_epoch = e1.max(e2);
    assert_eq!(max_epoch, 2);

    // The final state should reflect the last writer.
    let summary = handles
        .get_owned_summary("owner", "run-race")
        .await
        .unwrap();
    assert_eq!(summary.status, SubAgentStatus::Running);
}

#[tokio::test]
async fn concurrent_launches_different_run_ids() {
    let handles = Arc::new(SubAgentHandleTable::new());
    let num_runs = 10;
    let mut tasks = Vec::new();

    for i in 0..num_runs {
        let h = handles.clone();
        tasks.push(tokio::spawn(async move {
            h.put_running(
                &format!("run-{i}"),
                "owner".to_string(),
                format!("sub-agent-run-{i}"),
                "worker".to_string(),
                None,
                None,
            )
            .await
        }));
    }

    let epochs: Vec<u64> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All should be epoch 1 (first registration).
    for epoch in &epochs {
        assert_eq!(*epoch, 1);
    }

    // All 10 should be visible.
    let running = handles.running_or_stopped_for_owner("owner").await;
    assert_eq!(running.len(), num_runs);
    for summary in &running {
        assert_eq!(summary.status, SubAgentStatus::Running);
    }
}

#[tokio::test]
async fn concurrent_stop_multiple_independent_runs() {
    let handles = Arc::new(SubAgentHandleTable::new());

    // Register 5 independent runs.
    for i in 0..5 {
        handles
            .put_running(
                &format!("run-{i}"),
                "owner".to_string(),
                format!("sub-agent-run-{i}"),
                "worker".to_string(),
                None,
                None,
            )
            .await;
    }

    // Stop them all concurrently.
    let mut tasks = Vec::new();
    for i in 0..5 {
        let h = handles.clone();
        tasks.push(tokio::spawn(async move {
            h.stop_owned_tree("owner", &format!("run-{i}")).await
        }));
    }

    let results: Vec<_> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    for result in results {
        assert!(result.is_ok());
    }

    let all = handles.running_or_stopped_for_owner("owner").await;
    assert_eq!(all.len(), 5);
    for summary in &all {
        assert_eq!(summary.status, SubAgentStatus::Stopped);
    }
}

// ── Interleaved launch/stop/re-launch ────────────────────────────────────────

#[tokio::test]
async fn launch_stop_relaunch_increments_epoch_and_resumes() {
    let handles = SubAgentHandleTable::new();

    // Launch.
    let epoch1 = handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    assert_eq!(epoch1, 1);

    // Stop.
    handles.stop_owned_tree("owner", "run-1").await.unwrap();
    let summary = handles.get_owned_summary("owner", "run-1").await.unwrap();
    assert_eq!(summary.status, SubAgentStatus::Stopped);

    // Re-launch (simulates resume).
    let epoch2 = handles
        .put_running(
            "run-1",
            "owner".to_string(),
            "sub-agent-run-1".to_string(),
            "worker".to_string(),
            None,
            None,
        )
        .await;
    assert_eq!(epoch2, 2);

    let summary = handles.get_owned_summary("owner", "run-1").await.unwrap();
    assert_eq!(summary.status, SubAgentStatus::Running);

    // Stale epoch1 completion is ignored.
    let stale = handles
        .update_after_completion(
            "run-1",
            epoch1,
            SubAgentCompletion {
                status: SubAgentStatus::Completed,
                error: None,
            },
        )
        .await;
    assert!(stale.is_none());

    // Run is still running.
    let summary = handles.get_owned_summary("owner", "run-1").await.unwrap();
    assert_eq!(summary.status, SubAgentStatus::Running);

    // Correct epoch2 completion applies.
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
        .expect("epoch2 completion should apply");
    assert_eq!(applied.status, SubAgentStatus::Completed);
}

// ── Handle table: collect_descendant_run_ids_by_parent ───────────────────────

#[tokio::test]
async fn handle_table_collect_descendants_across_owners() {
    // Verifies that collect_descendant_run_ids_by_parent scopes to owner.
    let handles = SubAgentHandleTable::new();
    handles
        .put_running(
            "root",
            "owner-1".to_string(),
            "sub-agent-root".to_string(),
            "w".to_string(),
            None,
            None,
        )
        .await;
    handles
        .put_running(
            "child",
            "owner-1".to_string(),
            "sub-agent-child".to_string(),
            "w".to_string(),
            Some("root".to_string()),
            None,
        )
        .await;
    // This child belongs to a different owner, so should NOT be included.
    handles
        .put_running(
            "other-child",
            "owner-2".to_string(),
            "sub-agent-other-child".to_string(),
            "w".to_string(),
            Some("root".to_string()),
            None,
        )
        .await;

    let stopped = handles.stop_owned_tree("owner-1", "root").await.unwrap();
    let stopped_ids: Vec<&str> = stopped.iter().map(|s| s.run_id.as_str()).collect();
    assert!(stopped_ids.contains(&"root"));
    assert!(stopped_ids.contains(&"child"));
    assert!(!stopped_ids.contains(&"other-child"));

    // other-child should still be running.
    let other = handles
        .get_owned_summary("owner-2", "other-child")
        .await
        .unwrap();
    assert_eq!(other.status, SubAgentStatus::Running);
}
