use super::request::{convert_agui_messages, request_runtime_values};
use super::RunAgentRequest;
use crate::protocol::ProtocolInputAdapter;

pub struct AgUiInputAdapter;

impl ProtocolInputAdapter for AgUiInputAdapter {
    type Request = RunAgentRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest {
        let runtime = request_runtime_values(&request);

        crate::agent_os::RunRequest {
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
