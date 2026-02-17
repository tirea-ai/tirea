use super::AgentConfig;
use crate::contracts::runtime::StreamResult;
use crate::contracts::state::AgentState;
use crate::engine::stop_conditions::{condition_from_spec, StopCheckContext, StopCondition};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

/// Internal state tracked across run steps for stop condition evaluation.
pub(super) struct RunState {
    pub(super) completed_steps: usize,
    pub(super) total_input_tokens: usize,
    pub(super) total_output_tokens: usize,
    pub(super) consecutive_errors: usize,
    start_time: Instant,
    /// Tool call names per step (most recent last), capped at 20 entries.
    pub(super) tool_call_history: VecDeque<Vec<String>>,
}

impl RunState {
    pub(super) fn new() -> Self {
        Self {
            completed_steps: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            consecutive_errors: 0,
            start_time: Instant::now(),
            tool_call_history: VecDeque::new(),
        }
    }

    pub(super) fn update_from_response(&mut self, result: &StreamResult) {
        if let Some(ref usage) = result.usage {
            self.total_input_tokens += usage.prompt_tokens.unwrap_or(0) as usize;
            self.total_output_tokens += usage.completion_tokens.unwrap_or(0) as usize;
        }
    }

    pub(super) fn record_tool_step(
        &mut self,
        tool_calls: &[crate::contracts::state::ToolCall],
        error_count: usize,
    ) {
        let mut names: Vec<String> = tool_calls.iter().map(|tc| tc.name.clone()).collect();
        names.sort();
        if self.tool_call_history.len() >= 20 {
            self.tool_call_history.pop_front();
        }
        self.tool_call_history.push_back(names);

        if error_count > 0 && error_count == tool_calls.len() {
            self.consecutive_errors += 1;
        } else {
            self.consecutive_errors = 0;
        }
    }

    pub(super) fn to_check_context<'a>(
        &'a self,
        result: &'a StreamResult,
        thread: &'a AgentState,
    ) -> StopCheckContext<'a> {
        StopCheckContext {
            rounds: self.completed_steps,
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            consecutive_errors: self.consecutive_errors,
            elapsed: self.start_time.elapsed(),
            last_tool_calls: &result.tool_calls,
            last_text: &result.text,
            tool_call_history: &self.tool_call_history,
            thread,
        }
    }
}

/// Build the effective stop conditions for a run.
///
/// If the user explicitly configured stop conditions, use those.
/// Otherwise, create a default `MaxRounds` from `config.max_rounds`.
pub(super) fn effective_stop_conditions(config: &AgentConfig) -> Vec<Arc<dyn StopCondition>> {
    let mut conditions = config.stop_conditions.clone();
    for spec in &config.stop_condition_specs {
        conditions.push(condition_from_spec(spec.clone()));
    }
    if conditions.is_empty() {
        return vec![Arc::new(crate::engine::stop_conditions::MaxRounds(
            config.max_rounds,
        ))];
    }
    conditions
}
