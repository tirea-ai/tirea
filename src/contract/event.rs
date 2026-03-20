//! Agent loop streaming events.
//!
//! Simplified from uncarve's macro-generated definition. Wire format
//! (envelope, seq, timestamp) will be added when protocol adapters are built.

use super::inference::TokenUsage;
use super::lifecycle::TerminationReason;
use super::suspension::ToolCallOutcome;
use super::tool::ToolResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Agent loop events for streaming execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Run started.
    RunStart {
        thread_id: String,
        run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
    },

    /// Run finished.
    RunFinish {
        thread_id: String,
        run_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        termination: TerminationReason,
    },

    /// LLM text delta.
    TextDelta { delta: String },

    /// LLM reasoning delta.
    ReasoningDelta { delta: String },

    /// Tool call started.
    ToolCallStart { id: String, name: String },

    /// Tool call arguments delta.
    ToolCallDelta { id: String, args_delta: String },

    /// Tool call input is complete.
    ToolCallReady {
        id: String,
        name: String,
        arguments: Value,
    },

    /// Tool call completed.
    ToolCallDone {
        id: String,
        result: ToolResult,
        outcome: ToolCallOutcome,
    },

    /// Step started.
    StepStart {
        #[serde(default)]
        message_id: String,
    },

    /// Step completed.
    StepEnd,

    /// LLM inference completed.
    InferenceComplete {
        model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
        duration_ms: u64,
    },

    /// State snapshot.
    StateSnapshot { snapshot: Value },

    /// State delta.
    StateDelta { delta: Vec<Value> },

    /// Error occurred.
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl AgentEvent {
    /// Extract the response text from a `RunFinish` result value.
    pub fn extract_response(result: &Option<Value>) -> String {
        result
            .as_ref()
            .and_then(|v| v.get("response"))
            .and_then(|r| r.as_str())
            .unwrap_or_default()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_start_serde_roundtrip() {
        let event = AgentEvent::RunStart {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            parent_run_id: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event_type\":\"run_start\""));
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, AgentEvent::RunStart { .. }));
    }

    #[test]
    fn run_finish_serde_roundtrip() {
        let event = AgentEvent::RunFinish {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            result: Some(json!({"response": "hello"})),
            termination: TerminationReason::NaturalEnd,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        if let AgentEvent::RunFinish {
            result,
            termination,
            ..
        } = parsed
        {
            assert_eq!(AgentEvent::extract_response(&result), "hello");
            assert_eq!(termination, TerminationReason::NaturalEnd);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn text_delta_serde_roundtrip() {
        let event = AgentEvent::TextDelta {
            delta: "hello ".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, AgentEvent::TextDelta { delta } if delta == "hello "));
    }

    #[test]
    fn tool_call_done_serde_roundtrip() {
        let event = AgentEvent::ToolCallDone {
            id: "c1".into(),
            result: ToolResult::success("calc", json!(42)),
            outcome: ToolCallOutcome::Succeeded,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, AgentEvent::ToolCallDone { .. }));
    }

    #[test]
    fn step_end_serde_roundtrip() {
        let event = AgentEvent::StepEnd;
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, AgentEvent::StepEnd));
    }

    #[test]
    fn inference_complete_serde_roundtrip() {
        let event = AgentEvent::InferenceComplete {
            model: "gpt-4o".into(),
            usage: Some(TokenUsage {
                prompt_tokens: Some(100),
                completion_tokens: Some(50),
                total_tokens: Some(150),
                ..Default::default()
            }),
            duration_ms: 1234,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        if let AgentEvent::InferenceComplete {
            model,
            usage,
            duration_ms,
        } = parsed
        {
            assert_eq!(model, "gpt-4o");
            assert_eq!(usage.unwrap().total_tokens, Some(150));
            assert_eq!(duration_ms, 1234);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn error_event_serde_roundtrip() {
        let event = AgentEvent::Error {
            message: "something failed".into(),
            code: Some("INTERNAL".into()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, AgentEvent::Error { .. }));
    }

    #[test]
    fn extract_response_from_none() {
        assert_eq!(AgentEvent::extract_response(&None), "");
    }

    #[test]
    fn extract_response_from_missing_field() {
        assert_eq!(
            AgentEvent::extract_response(&Some(json!({"other": "data"}))),
            ""
        );
    }

    #[test]
    fn all_event_types_serialize() {
        let events: Vec<AgentEvent> = vec![
            AgentEvent::RunStart {
                thread_id: "t".into(),
                run_id: "r".into(),
                parent_run_id: None,
            },
            AgentEvent::RunFinish {
                thread_id: "t".into(),
                run_id: "r".into(),
                result: None,
                termination: TerminationReason::NaturalEnd,
            },
            AgentEvent::TextDelta { delta: "x".into() },
            AgentEvent::ReasoningDelta { delta: "y".into() },
            AgentEvent::ToolCallStart {
                id: "c".into(),
                name: "t".into(),
            },
            AgentEvent::ToolCallDelta {
                id: "c".into(),
                args_delta: "{}".into(),
            },
            AgentEvent::ToolCallReady {
                id: "c".into(),
                name: "t".into(),
                arguments: json!({}),
            },
            AgentEvent::ToolCallDone {
                id: "c".into(),
                result: ToolResult::success("t", json!(null)),
                outcome: ToolCallOutcome::Succeeded,
            },
            AgentEvent::StepStart {
                message_id: "m".into(),
            },
            AgentEvent::StepEnd,
            AgentEvent::InferenceComplete {
                model: "m".into(),
                usage: None,
                duration_ms: 0,
            },
            AgentEvent::StateSnapshot {
                snapshot: json!({}),
            },
            AgentEvent::StateDelta { delta: vec![] },
            AgentEvent::Error {
                message: "err".into(),
                code: None,
            },
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let _parsed: AgentEvent = serde_json::from_str(&json).unwrap();
        }
    }
}
