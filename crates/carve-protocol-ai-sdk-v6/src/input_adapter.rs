use carve_agent_runtime_contract::RunRequest;
use carve_protocol_contract::ProtocolInputAdapter;
use carve_thread_model::Message;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AiSdkV6RunRequest {
    #[serde(rename = "sessionId")]
    pub thread_id: String,
    pub input: String,
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
}

pub struct AiSdkV6InputAdapter;

impl ProtocolInputAdapter for AiSdkV6InputAdapter {
    type Request = AiSdkV6RunRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> RunRequest {
        RunRequest {
            agent_id,
            thread_id: if request.thread_id.trim().is_empty() {
                None
            } else {
                Some(request.thread_id)
            },
            run_id: request.run_id,
            resource_id: None,
            state: None,
            messages: vec![Message::user(request.input)],
            runtime: std::collections::HashMap::new(),
        }
    }
}
