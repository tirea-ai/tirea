use carve_agent::ag_ui::AGUIContext;
use carve_agent::{AGUIEvent, RunAgentRequest};

fn main() {
    let _ = std::mem::size_of::<AGUIContext>();
    let _ = std::mem::size_of::<AGUIEvent>();
    let _ = std::mem::size_of::<RunAgentRequest>();
}
