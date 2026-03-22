//! Control methods: cancel_by_thread, send_decisions.

use crate::contract::suspension::ToolCallResume;

use super::AgentRuntime;

impl AgentRuntime {
    /// Cancel an active run by thread ID.
    pub fn cancel_by_thread(&self, thread_id: &str) -> bool {
        if let Some(handle) = self.active_runs.get_handle(thread_id) {
            handle.cancel();
            true
        } else {
            false
        }
    }

    /// Send decisions to an active run by thread ID.
    pub fn send_decisions(
        &self,
        thread_id: &str,
        decisions: Vec<(String, ToolCallResume)>,
    ) -> bool {
        if let Some(handle) = self.active_runs.get_handle(thread_id) {
            for (call_id, resume) in decisions {
                if handle.send_decision(call_id, resume).is_err() {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
}
