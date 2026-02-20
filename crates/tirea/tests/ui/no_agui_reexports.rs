// Verify AG-UI types are NOT available without the "ag-ui" feature.
use tirea::ag_ui::AGUIContext;
use tirea::{AGUIEvent, RunAgentRequest};

fn main() {
    let _ = std::mem::size_of::<AGUIContext>();
    let _ = std::mem::size_of::<AGUIEvent>();
    let _ = std::mem::size_of::<RunAgentRequest>();
}
