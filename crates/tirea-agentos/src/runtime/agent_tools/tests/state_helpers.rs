use super::*;

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

#[test]
fn sub_agent_thread_id_convention() {
    assert_eq!(
        super::super::tools::sub_agent_thread_id("run-123"),
        "sub-agent-run-123"
    );
}

// ── State helper tests ───────────────────────────────────────────────────────

#[test]
fn collect_descendant_run_ids_from_state_includes_full_tree() {
    let runs = HashMap::from([
        (
            "root".to_string(),
            SubAgent {
                thread_id: "sub-agent-root".to_string(),
                parent_run_id: None,
                agent_id: "w".to_string(),
                status: SubAgentStatus::Running,
                error: None,
            },
        ),
        (
            "child-1".to_string(),
            SubAgent {
                thread_id: "sub-agent-child-1".to_string(),
                parent_run_id: Some("root".to_string()),
                agent_id: "w".to_string(),
                status: SubAgentStatus::Running,
                error: None,
            },
        ),
        (
            "child-2".to_string(),
            SubAgent {
                thread_id: "sub-agent-child-2".to_string(),
                parent_run_id: Some("root".to_string()),
                agent_id: "w".to_string(),
                status: SubAgentStatus::Running,
                error: None,
            },
        ),
        (
            "grandchild".to_string(),
            SubAgent {
                thread_id: "sub-agent-grandchild".to_string(),
                parent_run_id: Some("child-1".to_string()),
                agent_id: "w".to_string(),
                status: SubAgentStatus::Running,
                error: None,
            },
        ),
        (
            "unrelated".to_string(),
            SubAgent {
                thread_id: "sub-agent-unrelated".to_string(),
                parent_run_id: None,
                agent_id: "w".to_string(),
                status: SubAgentStatus::Running,
                error: None,
            },
        ),
    ]);

    let descendants = collect_descendant_run_ids_from_state(&runs, "root", true);
    assert!(descendants.contains(&"root".to_string()));
    assert!(descendants.contains(&"child-1".to_string()));
    assert!(descendants.contains(&"child-2".to_string()));
    assert!(descendants.contains(&"grandchild".to_string()));
    assert!(!descendants.contains(&"unrelated".to_string()));
    assert_eq!(descendants.len(), 4);
}

#[test]
fn collect_descendant_run_ids_from_state_exclude_root() {
    let runs = HashMap::from([
        (
            "root".to_string(),
            SubAgent {
                thread_id: "sub-agent-root".to_string(),
                parent_run_id: None,
                agent_id: "w".to_string(),
                status: SubAgentStatus::Running,
                error: None,
            },
        ),
        (
            "child".to_string(),
            SubAgent {
                thread_id: "sub-agent-child".to_string(),
                parent_run_id: Some("root".to_string()),
                agent_id: "w".to_string(),
                status: SubAgentStatus::Running,
                error: None,
            },
        ),
    ]);

    let descendants = collect_descendant_run_ids_from_state(&runs, "root", false);
    assert!(!descendants.contains(&"root".to_string()));
    assert!(descendants.contains(&"child".to_string()));
    assert_eq!(descendants.len(), 1);
}

#[test]
fn collect_descendant_run_ids_from_state_unknown_root_returns_empty() {
    let runs: HashMap<String, SubAgent> = HashMap::new();
    let descendants = collect_descendant_run_ids_from_state(&runs, "nonexistent", true);
    assert!(descendants.is_empty());
}

#[test]
fn collect_descendant_run_ids_from_state_leaf_node_returns_only_self() {
    let runs = HashMap::from([(
        "leaf".to_string(),
        SubAgent {
            thread_id: "sub-agent-leaf".to_string(),
            parent_run_id: Some("parent".to_string()),
            agent_id: "w".to_string(),
            status: SubAgentStatus::Running,
            error: None,
        },
    )]);

    let descendants = collect_descendant_run_ids_from_state(&runs, "leaf", true);
    assert_eq!(descendants, vec!["leaf".to_string()]);
}

// ── SubAgent serialization round-trip ────────────────────────────────────────

#[test]
fn sub_agent_serialization_roundtrip() {
    let sub = SubAgent {
        thread_id: "sub-agent-run-42".to_string(),
        parent_run_id: Some("parent-7".to_string()),
        agent_id: "researcher".to_string(),
        status: SubAgentStatus::Completed,
        error: None,
    };
    let json = serde_json::to_value(&sub).unwrap();
    assert_eq!(json["thread_id"], "sub-agent-run-42");
    assert_eq!(json["parent_run_id"], "parent-7");
    assert_eq!(json["agent_id"], "researcher");
    assert_eq!(json["status"], "completed");
    assert!(json.get("error").is_none(), "None error should be skipped");

    let roundtrip: SubAgent = serde_json::from_value(json).unwrap();
    assert_eq!(roundtrip.thread_id, sub.thread_id);
    assert_eq!(roundtrip.parent_run_id, sub.parent_run_id);
    assert_eq!(roundtrip.agent_id, sub.agent_id);
    assert_eq!(roundtrip.status as u8, SubAgentStatus::Completed as u8);
    assert!(roundtrip.error.is_none());
}

#[test]
fn sub_agent_deserializes_without_optional_fields() {
    let json = json!({
        "thread_id": "sub-agent-run-99",
        "agent_id": "coder",
        "status": "running"
    });
    let sub: SubAgent = serde_json::from_value(json).unwrap();
    assert!(sub.parent_run_id.is_none());
    assert!(sub.error.is_none());
}

#[test]
fn sub_agent_status_as_str() {
    assert_eq!(SubAgentStatus::Running.as_str(), "running");
    assert_eq!(SubAgentStatus::Completed.as_str(), "completed");
    assert_eq!(SubAgentStatus::Failed.as_str(), "failed");
    assert_eq!(SubAgentStatus::Stopped.as_str(), "stopped");
}
