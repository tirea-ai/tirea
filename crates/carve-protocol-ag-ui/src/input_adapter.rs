use super::request::convert_agui_messages;
use super::RunAgentRequest;
use carve_agent_contract::{ProtocolInputAdapter, RunRequest};

pub struct AgUiInputAdapter;

impl ProtocolInputAdapter for AgUiInputAdapter {
    type Request = RunAgentRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> RunRequest {
        RunRequest {
            agent_id,
            thread_id: Some(request.thread_id),
            run_id: Some(request.run_id),
            parent_run_id: request.parent_run_id,
            resource_id: None,
            state: request.state,
            messages: convert_agui_messages(&request.messages),
        }
    }
}
