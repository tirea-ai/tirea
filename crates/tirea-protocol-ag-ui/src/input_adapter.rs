use super::RunAgentRequest;
use tirea_contract::{ProtocolInputAdapter, RunRequest};

pub struct AgUiInputAdapter;

impl ProtocolInputAdapter for AgUiInputAdapter {
    type Request = RunAgentRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> RunRequest {
        request.into_runtime_run_request(agent_id)
    }
}
