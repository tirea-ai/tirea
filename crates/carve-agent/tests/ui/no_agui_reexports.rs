// Verify AG-UI types are NOT available without the "ag-ui" feature.
use carve_agent::ag_ui::AGUIContext;
use carve_agent::{AGUIEvent, RunAgentRequest};

fn main() {
    let _ = std::mem::size_of::<AGUIContext>();
    let _ = std::mem::size_of::<AGUIEvent>();
    let _ = std::mem::size_of::<RunAgentRequest>();
}
