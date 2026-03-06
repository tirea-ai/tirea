use crate::runtime::run::TerminationReason;
use crate::thread::Message;

/// Flow control extension: run-level action.
///
/// Populated by `RequestTermination` action.
#[derive(Debug, Default, Clone)]
pub struct FlowControl {
    /// Run-level action emitted by plugins.
    pub run_action: Option<RunAction>,
}

/// Run-level control action emitted by plugins.
#[derive(Debug, Clone)]
pub enum RunAction {
    /// Continue normal execution.
    Continue,
    /// Terminate run with specific reason.
    Terminate(TerminationReason),
    /// Re-run inference after appending messages.
    RetryInference {
        /// Messages to append before retrying.
        messages: Vec<Message>,
    },
}
