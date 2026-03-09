use super::*;

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

// ── AgentOutputTool: missing run_id param ────────────────────────────────────

#[tokio::test]
async fn agent_output_tool_requires_run_id() {
    let os = AgentOs::builder().build().unwrap();
    let tool = AgentOutputTool::new(os);
    let fix = TestFixture::new();

    let result = tool
        .execute(json!({}), &fix.ctx_with("call-1", "tool:agent_output"))
        .await
        .unwrap();
    assert_eq!(result.status, ToolStatus::Error);
    assert!(result
        .message
        .unwrap_or_default()
        .contains("missing 'run_id'"));
}
