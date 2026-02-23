#![allow(missing_docs)]

use serde_json::{json, Value};
use tirea_protocol_ag_ui::{
    convert_agui_messages, Message, Role, RunAgentInput, Tool, ToolExecutionLocation,
};

#[test]
fn frontend_tools_filters_backend_tools() {
    let request = RunAgentInput {
        tools: vec![
            Tool::backend("search", "backend"),
            Tool::frontend("copy_to_clipboard", "frontend"),
            Tool {
                name: "open_modal".to_string(),
                description: "frontend by default marker".to_string(),
                parameters: None,
                execute: ToolExecutionLocation::Frontend,
            },
        ],
        ..RunAgentInput::new("thread_1", "run_1")
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
    let request = RunAgentInput::new("thread_1", "run_1")
        .with_message(Message::user("hello"))
        .with_message(Message::assistant("ignored"))
        .with_message(Message {
            role: Role::Tool,
            content: "true".to_string(),
            id: None,
            tool_call_id: None,
        })
        .with_message(Message::tool("false", "interaction_1"));

    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].interaction_id, "interaction_1");
    assert_eq!(responses[0].result, Value::Bool(false));
    assert!(request.has_any_interaction_responses());
}

#[test]
fn has_user_input_only_counts_non_empty_user_messages() {
    let no_user = RunAgentInput::new("thread_1", "run_1")
        .with_message(Message::assistant("ignored"))
        .with_message(Message::tool("true", "interaction_1"));
    assert!(!no_user.has_user_input());

    let empty_user = RunAgentInput::new("thread_1", "run_1").with_message(Message::user("   "));
    assert!(!empty_user.has_user_input());

    let with_user = RunAgentInput::new("thread_1", "run_1")
        .with_message(Message::user("hello"))
        .with_message(Message::tool("true", "interaction_1"));
    assert!(with_user.has_user_input());
}

#[test]
fn interaction_response_non_json_content_is_preserved_as_string() {
    let request =
        RunAgentInput::new("thread_1", "run_1").with_message(Message::tool("approved", "i1"));

    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result, Value::String("approved".to_string()));
}

#[test]
fn approved_and_denied_ids_follow_runtime_interaction_semantics() {
    let request = RunAgentInput::new("thread_1", "run_1")
        .with_message(Message::tool("true", "approved_bool"))
        .with_message(Message::tool(r#"{"allowed": true}"#, "approved_object"))
        .with_message(Message::tool(r#"{"approved": false}"#, "denied_object"))
        .with_message(Message::tool(
            r#"{"status":"cancelled"}"#,
            "cancelled_object",
        ))
        .with_message(Message::tool("no", "denied_string"))
        .with_message(Message::tool("cancelled", "cancelled_string"))
        .with_message(Message::tool("maybe", "neither"));

    assert_eq!(
        request.approved_interaction_ids(),
        vec!["approved_bool", "approved_object"]
    );
    assert_eq!(
        request.denied_interaction_ids(),
        vec![
            "denied_object",
            "cancelled_object",
            "denied_string",
            "cancelled_string"
        ]
    );
}

#[test]
fn conflicting_results_for_same_interaction_id_use_last_result() {
    let request = RunAgentInput::new("thread_1", "run_1")
        .with_message(Message::tool("true", "same_id"))
        .with_message(Message::tool("false", "same_id"));

    assert!(request.approved_interaction_ids().is_empty());
    assert_eq!(request.denied_interaction_ids(), vec!["same_id"]);
}

#[test]
fn interaction_responses_filter_to_pending_ids_when_state_exists() {
    let request = RunAgentInput::new("thread_1", "run_1")
        .with_state(json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_pending": {
                        "call_id": "call_pending",
                        "tool_name": "confirm",
                        "suspension": { "id": "call_pending", "action": "confirm" },
                        "invocation": { "call_id": "call_pending" }
                    }
                }
            }
        }))
        .with_message(Message::tool("true", "call_pending"))
        .with_message(Message::tool("true", "unrelated_tool_result"));

    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].interaction_id, "call_pending");
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

    let request = RunAgentInput::new("thread_1", "run_1")
        .with_message(Message::tool(payload.to_string(), "interaction_1"));

    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result, payload);
}

#[test]
fn run_request_deserializes_forwarded_props_aliases() {
    let camel: RunAgentInput = serde_json::from_value(json!({
        "threadId": "t1",
        "runId": "r1",
        "messages": [],
        "forwardedProps": { "foo": 1 }
    }))
    .unwrap();
    assert_eq!(camel.forwarded_props, Some(json!({ "foo": 1 })));

    let snake: RunAgentInput = serde_json::from_value(json!({
        "threadId": "t1",
        "runId": "r1",
        "messages": [],
        "forwarded_props": { "bar": 2 }
    }))
    .unwrap();
    assert_eq!(snake.forwarded_props, Some(json!({ "bar": 2 })));
}

#[test]
fn convert_agui_messages_filters_activity_and_reasoning() {
    let input = vec![
        Message::user("hello"),
        Message::activity("status"),
        Message::reasoning("thought"),
    ];

    let messages = convert_agui_messages(&input);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "hello");
}
