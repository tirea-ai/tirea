use super::request::{convert_agui_messages, request_runtime_values};
use super::RunAgentRequest;
use carve_agent_runtime_contract::RunRequest;
use carve_protocol_contract::ProtocolInputAdapter;

pub struct AgUiInputAdapter;

impl ProtocolInputAdapter for AgUiInputAdapter {
    type Request = RunAgentRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> RunRequest {
        let runtime = request_runtime_values(&request);

        RunRequest {
            agent_id,
            thread_id: Some(request.thread_id),
            run_id: Some(request.run_id),
            resource_id: None,
            state: request.state,
            messages: convert_agui_messages(&request.messages),
            runtime,
        }
    }
}
