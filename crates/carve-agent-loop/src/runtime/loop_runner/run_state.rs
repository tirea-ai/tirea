use super::outcome::{LoopStats, LoopUsage};
use super::AgentConfig;
use crate::contracts::runtime::StreamResult;
use crate::contracts::state::AgentState;
use crate::engine::stop_conditions::{
    condition_from_spec, StopCondition, StopPolicyInput, StopPolicyStats,
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

/// Internal state tracked across run steps for stop condition evaluation.
pub(super) struct RunState {
    pub(super) completed_steps: usize,
    pub(super) total_input_tokens: usize,
    pub(super) total_output_tokens: usize,
    pub(super) llm_calls: usize,
    pub(super) llm_retries: usize,
    pub(super) tool_calls: usize,
    pub(super) step_tool_call_count: usize,
    pub(super) tool_errors: usize,
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
            llm_calls: 0,
            llm_retries: 0,
            tool_calls: 0,
            step_tool_call_count: 0,
            tool_errors: 0,
            consecutive_errors: 0,
            start_time: Instant::now(),
            tool_call_history: VecDeque::new(),
        }
    }

    pub(super) fn record_llm_attempts(&mut self, attempts: usize) {
        if attempts == 0 {
            return;
        }
        self.llm_calls = self.llm_calls.saturating_add(attempts);
        self.llm_retries = self.llm_retries.saturating_add(attempts.saturating_sub(1));
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
        self.step_tool_call_count = tool_calls.len();
        let mut names: Vec<String> = tool_calls.iter().map(|tc| tc.name.clone()).collect();
        names.sort();
        if self.tool_call_history.len() >= 20 {
            self.tool_call_history.pop_front();
        }
        self.tool_call_history.push_back(names);
        self.tool_calls = self.tool_calls.saturating_add(tool_calls.len());
        self.tool_errors = self.tool_errors.saturating_add(error_count);

        if error_count > 0 && error_count == tool_calls.len() {
            self.consecutive_errors += 1;
        } else {
            self.consecutive_errors = 0;
        }
    }

    pub(super) fn record_step_without_tools(&mut self) {
        self.step_tool_call_count = 0;
    }

    pub(super) fn usage(&self) -> LoopUsage {
        LoopUsage {
            prompt_tokens: self.total_input_tokens,
            completion_tokens: self.total_output_tokens,
            total_tokens: self.total_input_tokens + self.total_output_tokens,
        }
    }

    pub(super) fn stats(&self) -> LoopStats {
        LoopStats {
            duration_ms: self.start_time.elapsed().as_millis() as u64,
            steps: self.completed_steps,
            llm_calls: self.llm_calls,
            llm_retries: self.llm_retries,
            tool_calls: self.tool_calls,
            tool_errors: self.tool_errors,
        }
    }

    pub(super) fn to_policy_input<'a>(
        &'a self,
        result: &'a StreamResult,
        thread: &'a AgentState,
    ) -> StopPolicyInput<'a> {
        StopPolicyInput {
            agent_state: thread,
            stats: StopPolicyStats {
                step: self.completed_steps,
                step_tool_call_count: self.step_tool_call_count,
                total_tool_call_count: self.tool_calls,
                total_input_tokens: self.total_input_tokens,
                total_output_tokens: self.total_output_tokens,
                consecutive_errors: self.consecutive_errors,
                elapsed: self.start_time.elapsed(),
                last_tool_calls: &result.tool_calls,
                last_text: &result.text,
                tool_call_history: &self.tool_call_history,
            },
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
