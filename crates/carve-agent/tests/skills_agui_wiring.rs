use carve_agent::{
    execute_single_tool, AGUIContext, AGUIEvent, AgentEvent, FsSkillRegistry, Session,
    SkillSubsystem, ToolCall, ToolDescriptor,
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[tokio::test]
async fn test_skill_tool_result_is_emitted_as_agui_tool_call_result() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("skills");
    fs::create_dir_all(root.join("docx")).unwrap();
    fs::write(
        root.join("docx").join("SKILL.md"),
        "---\nname: docx\ndescription: docx\n---\nUse docx-js.\n",
    )
    .unwrap();

    let skills = SkillSubsystem::new(std::sync::Arc::new(
        FsSkillRegistry::discover_root(root).unwrap(),
    ));
    let tools = skills.tools();
    let tool = tools.get("skill").expect("skill tool registered");

    let session = Session::with_initial_state("s", json!({}));
    let state = session.rebuild_state().unwrap();
    let call = ToolCall::new("call_1", "skill", json!({"skill": "docx"}));

    let exec = execute_single_tool(Some(tool.as_ref()), &call, &state).await;
    assert!(exec.result.is_success());

    // Simulate tool call lifecycle events being converted to AG-UI.
    let mut agui_ctx = AGUIContext::new("thread_1".to_string(), "run_1".to_string());
    let start = AgentEvent::ToolCallStart {
        id: "call_1".to_string(),
        name: "skill".to_string(),
    };
    let ready = AgentEvent::ToolCallReady {
        id: "call_1".to_string(),
        name: "skill".to_string(),
        arguments: json!({"skill": "docx"}),
    };
    let delta = AgentEvent::ToolCallDelta {
        id: "call_1".to_string(),
        args_delta: "{\"skill\":\"docx\"}".to_string(),
    };
    let done = AgentEvent::ToolCallDone {
        id: "call_1".to_string(),
        result: exec.result.clone(),
        patch: exec.patch.clone(),
    };

    let mut events = Vec::new();
    events.extend(start.to_ag_ui_events(&mut agui_ctx));
    events.extend(delta.to_ag_ui_events(&mut agui_ctx));
    events.extend(ready.to_ag_ui_events(&mut agui_ctx));
    events.extend(done.to_ag_ui_events(&mut agui_ctx));

    assert!(events
        .iter()
        .any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, AGUIEvent::ToolCallArgs { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, AGUIEvent::ToolCallEnd { .. })));

    let args_event = events
        .iter()
        .find_map(|e| {
            if let AGUIEvent::ToolCallArgs {
                tool_call_id,
                delta,
                ..
            } = e
            {
                Some((tool_call_id.clone(), delta.clone()))
            } else {
                None
            }
        })
        .expect("ToolCallArgs emitted");
    assert_eq!(args_event.0, "call_1");
    assert!(args_event.1.contains("\"skill\""));
    assert!(args_event.1.contains("\"docx\""));

    let result_event = events
        .iter()
        .find_map(|e| {
            if let AGUIEvent::ToolCallResult {
                tool_call_id,
                content,
                ..
            } = e
            {
                Some((tool_call_id.clone(), content.clone()))
            } else {
                None
            }
        })
        .expect("ToolCallResult emitted");

    assert_eq!(result_event.0, "call_1");
    let parsed: serde_json::Value = serde_json::from_str(&result_event.1).expect("json content");
    assert_eq!(parsed["tool_name"], "skill");
    assert_eq!(parsed["status"], "success");
    assert_eq!(parsed["data"]["activated"], true);
    assert_eq!(parsed["data"]["skill_id"], "docx");
}

#[tokio::test]
async fn test_skills_plugin_injection_is_in_system_context_before_inference() {
    let td = TempDir::new().unwrap();
    let root = td.path().join("skills");
    fs::create_dir_all(root.join("docx")).unwrap();
    fs::write(
        root.join("docx").join("SKILL.md"),
        "---\nname: docx\ndescription: docx\n---\nBody\n",
    )
    .unwrap();

    let skills = SkillSubsystem::new(std::sync::Arc::new(
        FsSkillRegistry::discover_root(root).unwrap(),
    ));
    let plugin = skills.plugin();

    // Even without activation, discovery should inject available_skills.
    let session = Session::with_initial_state("s", json!({}));
    let mut step =
        carve_agent::StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
    plugin
        .on_phase(carve_agent::Phase::BeforeInference, &mut step)
        .await;
    assert_eq!(step.system_context.len(), 1);
    assert!(step.system_context[0].contains("<available_skills>"));
}
