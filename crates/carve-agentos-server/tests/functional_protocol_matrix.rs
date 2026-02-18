use carve_agent::contracts::Role;
use carve_protocol_ag_ui::{
    convert_agui_messages, AGUIMessage, MessageRole, RunAgentRequest,
};
use carve_protocol_ai_sdk_v6::{AiSdkV6InputAdapter, AiSdkV6RunRequest};
use carve_protocol_contract::ProtocolInputAdapter;

#[test]
fn functional_protocol_scenario_matrix_180() {
    let mut executed = 0usize;

    // ---------------------------------------------------------------------
    // 1) AG-UI request validation matrix (36 scenarios)
    // ---------------------------------------------------------------------
    let thread_cases = ["thread-1", "", " ", "thread_x", "0", "\t"];
    let run_cases = ["run-1", "", " ", "run_x", "0", "\t"];

    for thread_id in thread_cases {
        for run_id in run_cases {
            let req = RunAgentRequest::new(thread_id.to_string(), run_id.to_string());
            let expected_ok = !thread_id.is_empty() && !run_id.is_empty();
            assert_eq!(
                req.validate().is_ok(),
                expected_ok,
                "validation mismatch for thread_id={thread_id:?}, run_id={run_id:?}"
            );
            executed += 1;
        }
    }

    // ---------------------------------------------------------------------
    // 2) AI SDK input adapter matrix (84 scenarios)
    // ---------------------------------------------------------------------
    let aisdk_thread_cases = ["thread-1", "", " ", "\t", "thread-42", "x", " session "];
    let aisdk_run_cases = [None, Some("run-a"), Some("run-b")];
    let aisdk_input_cases = ["hello", "hi there", "42", "{\"a\":1}"];

    for thread_id in aisdk_thread_cases {
        for run_id in aisdk_run_cases {
            for input in aisdk_input_cases {
                let req = AiSdkV6RunRequest {
                    thread_id: thread_id.to_string(),
                    input: input.to_string(),
                    run_id: run_id.map(str::to_string),
                };
                let run = AiSdkV6InputAdapter::to_run_request("agent".to_string(), req);
                let expected_thread = if thread_id.trim().is_empty() {
                    None
                } else {
                    Some(thread_id.to_string())
                };

                assert_eq!(run.thread_id, expected_thread);
                assert_eq!(run.run_id, run_id.map(str::to_string));
                assert_eq!(run.messages.len(), 1);
                assert_eq!(run.messages[0].role, Role::User);
                assert_eq!(run.messages[0].content, input);
                executed += 1;
            }
        }
    }

    // ---------------------------------------------------------------------
    // 3) AG-UI message conversion matrix (60 scenarios)
    // ---------------------------------------------------------------------
    let role_cases = [
        MessageRole::Developer,
        MessageRole::System,
        MessageRole::User,
        MessageRole::Assistant,
        MessageRole::Tool,
    ];
    let id_cases = [false, true];
    let tool_call_cases = [false, true];
    let content_cases = ["plain text", "", "{\"ok\":true}"];

    for role in role_cases {
        for has_id in id_cases {
            for has_tool_call_id in tool_call_cases {
                for content in content_cases {
                    let msg = AGUIMessage {
                        role: role.clone(),
                        content: content.to_string(),
                        id: has_id.then(|| "msg-fixed-id".to_string()),
                        tool_call_id: has_tool_call_id.then(|| "call-fixed-id".to_string()),
                    };
                    let converted = convert_agui_messages(&[msg]);

                    if role == MessageRole::Assistant {
                        assert!(
                            converted.is_empty(),
                            "assistant messages must be filtered out"
                        );
                    } else {
                        assert_eq!(converted.len(), 1);
                        let core = &converted[0];
                        let expected_role = match role {
                            MessageRole::Developer | MessageRole::System => Role::System,
                            MessageRole::User => Role::User,
                            MessageRole::Tool => Role::Tool,
                            MessageRole::Assistant => unreachable!(),
                        };
                        assert_eq!(core.role, expected_role);
                        assert_eq!(core.content, content);
                        if has_id {
                            assert_eq!(core.id.as_deref(), Some("msg-fixed-id"));
                        } else {
                            assert!(core.id.as_ref().is_some_and(|id| !id.is_empty()));
                        }
                        assert_eq!(
                            core.tool_call_id.as_deref(),
                            has_tool_call_id.then_some("call-fixed-id")
                        );
                    }

                    executed += 1;
                }
            }
        }
    }

    assert_eq!(executed, 180, "functional scenario count drifted");
}
