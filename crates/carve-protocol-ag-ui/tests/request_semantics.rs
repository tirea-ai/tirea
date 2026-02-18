#![allow(missing_docs)]

use carve_protocol_ag_ui::{
    AGUIMessage, AGUIToolDef, MessageRole, RunAgentRequest, ToolExecutionLocation,
};
use serde_json::{json, Value};

#[test]
fn frontend_tools_filters_backend_tools() {
    let request = RunAgentRequest {
        tools: vec![
            AGUIToolDef::backend("search", "backend"),
            AGUIToolDef::frontend("copy_to_clipboard", "frontend"),
            AGUIToolDef {
                name: "open_modal".to_string(),
                description: "frontend by default marker".to_string(),
                parameters: None,
                execute: ToolExecutionLocation::Frontend,
            },
        ],
        ..RunAgentRequest::new("thread_1", "run_1")
    };

    let frontend: Vec<String> = request
        .frontend_tools()
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert_eq!(frontend, vec!["copy_to_clipboard", "open_modal"]);
}

#[test]
fn interaction_responses_ignore_non_tool_messages_and_tool_without_id() {
    let request = RunAgentRequest::new("thread_1", "run_1")
        .with_message(AGUIMessage::user("hello"))
        .with_message(AGUIMessage::assistant("ignored"))
        .with_message(AGUIMessage {
            role: MessageRole::Tool,
            content: "true".to_string(),
            id: None,
            tool_call_id: None,
        })
        .with_message(AGUIMessage::tool("false", "interaction_1"));

    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].interaction_id, "interaction_1");
    assert_eq!(responses[0].result, Value::Bool(false));
    assert!(request.has_any_interaction_responses());
}

#[test]
fn interaction_response_non_json_content_is_preserved_as_string() {
    let request =
        RunAgentRequest::new("thread_1", "run_1").with_message(AGUIMessage::tool("approved", "i1"));

    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result, Value::String("approved".to_string()));
}

#[test]
fn approved_and_denied_ids_follow_runtime_interaction_semantics() {
    let request = RunAgentRequest::new("thread_1", "run_1")
        .with_message(AGUIMessage::tool("true", "approved_bool"))
        .with_message(AGUIMessage::tool(r#"{"allowed": true}"#, "approved_object"))
        .with_message(AGUIMessage::tool(r#"{"approved": false}"#, "denied_object"))
        .with_message(AGUIMessage::tool("no", "denied_string"))
        .with_message(AGUIMessage::tool("maybe", "neither"));

    assert_eq!(
        request.approved_interaction_ids(),
        vec!["approved_bool", "approved_object"]
    );
    assert_eq!(
        request.denied_interaction_ids(),
        vec!["denied_object", "denied_string"]
    );
}

#[test]
fn conflicting_results_for_same_interaction_id_appear_in_both_lists() {
    let request = RunAgentRequest::new("thread_1", "run_1")
        .with_message(AGUIMessage::tool("true", "same_id"))
        .with_message(AGUIMessage::tool("false", "same_id"));

    assert_eq!(request.approved_interaction_ids(), vec!["same_id"]);
    assert_eq!(request.denied_interaction_ids(), vec!["same_id"]);
}

#[test]
fn interaction_response_preserves_json_values() {
    let payload = json!({
        "approved": true,
        "meta": {
            "source": "client",
            "reason": "user_confirmed"
        }
    });

    let request = RunAgentRequest::new("thread_1", "run_1")
        .with_message(AGUIMessage::tool(payload.to_string(), "interaction_1"));

    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result, payload);
}
