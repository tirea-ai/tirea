use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::contracts::plugin::phase::{AfterInferenceContext, PluginPhaseContext};
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::runtime::{StopPolicy, StopPolicyInput, StopPolicyStats, StreamResult};
use crate::contracts::thread::{Message, Role, ToolCall};
use crate::contracts::{RunContext, StopConditionSpec, ToolResult};
use tirea_state::State;

pub const STOP_POLICY_PLUGIN_ID: &str = "stop_policy";

#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "__kernel.stop_policy_runtime")]
struct StopPolicyRuntimeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub total_input_tokens: usize,
    #[serde(default)]
    pub total_output_tokens: usize,
}

#[derive(Debug, Clone, Default)]
struct MessageDerivedStopStats {
    step: usize,
    step_tool_call_count: usize,
    total_tool_call_count: usize,
    consecutive_errors: usize,
    last_tool_calls: Vec<ToolCall>,
    last_text: String,
    tool_call_history: VecDeque<Vec<String>>,
}

/// Plugin adapter that evaluates configured stop policies at `AfterInference`.
///
/// This keeps stop-domain semantics out of the core loop.
pub struct StopPolicyPlugin {
    conditions: Vec<Arc<dyn StopPolicy>>,
}

impl std::fmt::Debug for StopPolicyPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopPolicyPlugin")
            .field("conditions_len", &self.conditions.len())
            .finish()
    }
}

impl StopPolicyPlugin {
    pub fn new(
        mut stop_conditions: Vec<Arc<dyn StopPolicy>>,
        stop_condition_specs: Vec<StopConditionSpec>,
    ) -> Self {
        stop_conditions.extend(stop_condition_specs.into_iter().map(condition_from_spec));
        Self {
            conditions: stop_conditions,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }
}

#[async_trait]
impl AgentPlugin for StopPolicyPlugin {
    fn id(&self) -> &str {
        STOP_POLICY_PLUGIN_ID
    }

    async fn after_inference(&self, step: &mut AfterInferenceContext<'_, '_>) {
        if self.conditions.is_empty() {
            return;
        }

        let Some(response) = step.response_opt() else {
            return;
        };
        let now_ms = now_millis();
        let prompt_tokens = response
            .usage
            .as_ref()
            .and_then(|usage| usage.prompt_tokens)
            .unwrap_or(0) as usize;
        let completion_tokens = response
            .usage
            .as_ref()
            .and_then(|usage| usage.completion_tokens)
            .unwrap_or(0) as usize;

        let (started_at_ms, total_input_tokens, total_output_tokens) = {
            let runtime = step.state_of::<StopPolicyRuntimeState>();
            let started_at_ms = runtime.started_at_ms().ok().flatten().unwrap_or(now_ms);
            if runtime.started_at_ms().ok().flatten().is_none() {
                let _ = runtime.set_started_at_ms(Some(now_ms));
            }

            let current_input = runtime.total_input_tokens().ok().unwrap_or(0);
            let current_output = runtime.total_output_tokens().ok().unwrap_or(0);
            let total_input_tokens = current_input.saturating_add(prompt_tokens);
            let total_output_tokens = current_output.saturating_add(completion_tokens);
            if prompt_tokens > 0 {
                let _ = runtime.set_total_input_tokens(total_input_tokens);
            }
            if completion_tokens > 0 {
                let _ = runtime.set_total_output_tokens(total_output_tokens);
            }
            (started_at_ms, total_input_tokens, total_output_tokens)
        };

        let message_stats = derive_stats_from_messages_with_response(step.messages(), response);
        let elapsed = std::time::Duration::from_millis(now_ms.saturating_sub(started_at_ms));

        let run_ctx = RunContext::new(
            step.thread_id().to_string(),
            step.snapshot(),
            step.messages().to_vec(),
            step.run_config().clone(),
        );
        let input = StopPolicyInput {
            agent_state: &run_ctx,
            stats: StopPolicyStats {
                step: message_stats.step,
                step_tool_call_count: message_stats.step_tool_call_count,
                total_tool_call_count: message_stats.total_tool_call_count,
                total_input_tokens,
                total_output_tokens,
                consecutive_errors: message_stats.consecutive_errors,
                elapsed,
                last_tool_calls: &message_stats.last_tool_calls,
                last_text: &message_stats.last_text,
                tool_call_history: &message_stats.tool_call_history,
            },
        };
        for condition in &self.conditions {
            if let Some(reason) = condition.evaluate(&input) {
                step.request_termination(crate::contracts::TerminationReason::Stopped(reason));
                break;
            }
        }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn derive_stats_from_messages(messages: &[Arc<Message>]) -> MessageDerivedStopStats {
    let mut assistant_indices = Vec::new();
    for (idx, message) in messages.iter().enumerate() {
        if message.role == Role::Assistant {
            assistant_indices.push(idx);
        }
    }

    let mut stats = MessageDerivedStopStats {
        step: assistant_indices.len(),
        ..MessageDerivedStopStats::default()
    };
    let mut consecutive_errors = 0usize;

    for (round_idx, &assistant_idx) in assistant_indices.iter().enumerate() {
        let assistant = &messages[assistant_idx];
        let tool_calls = assistant.tool_calls.clone().unwrap_or_default();

        if !tool_calls.is_empty() {
            stats.total_tool_call_count =
                stats.total_tool_call_count.saturating_add(tool_calls.len());
            let mut names: Vec<String> = tool_calls.iter().map(|tc| tc.name.clone()).collect();
            names.sort();
            if stats.tool_call_history.len() >= 20 {
                stats.tool_call_history.pop_front();
            }
            stats.tool_call_history.push_back(names);
        }

        if round_idx + 1 == assistant_indices.len() {
            stats.step_tool_call_count = tool_calls.len();
            stats.last_tool_calls = tool_calls.clone();
            stats.last_text = assistant.content.clone();
        }

        if tool_calls.is_empty() {
            consecutive_errors = 0;
            continue;
        }

        let next_assistant_idx = assistant_indices
            .get(round_idx + 1)
            .copied()
            .unwrap_or(messages.len());
        let tool_results =
            collect_round_tool_results(messages, assistant_idx + 1, next_assistant_idx);
        let round_all_errors = tool_calls
            .iter()
            .all(|call| tool_results.get(&call.id).copied().unwrap_or(false));
        if round_all_errors {
            consecutive_errors = consecutive_errors.saturating_add(1);
        } else {
            consecutive_errors = 0;
        }
    }

    stats.consecutive_errors = consecutive_errors;
    stats
}

fn derive_stats_from_messages_with_response(
    messages: &[Arc<Message>],
    response: &StreamResult,
) -> MessageDerivedStopStats {
    let mut all_messages = Vec::with_capacity(messages.len() + 1);
    all_messages.extend(messages.iter().cloned());
    all_messages.push(Arc::new(Message::assistant_with_tool_calls(
        response.text.clone(),
        response.tool_calls.clone(),
    )));
    derive_stats_from_messages(&all_messages)
}

fn collect_round_tool_results(
    messages: &[Arc<Message>],
    from: usize,
    to: usize,
) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    for message in messages.iter().take(to).skip(from) {
        if message.role != Role::Tool {
            continue;
        }
        let Some(call_id) = message.tool_call_id.as_ref() else {
            continue;
        };
        let is_error = serde_json::from_str::<ToolResult>(&message.content)
            .map(|result| result.is_error())
            .unwrap_or(false);
        out.insert(call_id.clone(), is_error);
    }
    out
}

fn condition_from_spec(spec: StopConditionSpec) -> Arc<dyn StopPolicy> {
    match spec {
        StopConditionSpec::MaxRounds { rounds } => {
            Arc::new(crate::engine::stop_conditions::MaxRounds(rounds))
        }
        StopConditionSpec::Timeout { seconds } => Arc::new(
            crate::engine::stop_conditions::Timeout(std::time::Duration::from_secs(seconds)),
        ),
        StopConditionSpec::TokenBudget { max_total } => {
            Arc::new(crate::engine::stop_conditions::TokenBudget { max_total })
        }
        StopConditionSpec::ConsecutiveErrors { max } => {
            Arc::new(crate::engine::stop_conditions::ConsecutiveErrors(max))
        }
        StopConditionSpec::StopOnTool { tool_name } => {
            Arc::new(crate::engine::stop_conditions::StopOnTool(tool_name))
        }
        StopConditionSpec::ContentMatch { pattern } => {
            Arc::new(crate::engine::stop_conditions::ContentMatch(pattern))
        }
        StopConditionSpec::LoopDetection { window } => {
            Arc::new(crate::engine::stop_conditions::LoopDetection { window })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::thread::Message;
    use crate::contracts::StreamResult;
    use serde_json::json;

    #[test]
    fn derives_round_stats_from_messages() {
        let call_1 = ToolCall::new("c1", "failing", json!({}));
        let call_2 = ToolCall::new("c2", "echo", json!({}));
        let messages = vec![
            Arc::new(Message::assistant_with_tool_calls(
                "r1",
                vec![call_1.clone()],
            )),
            Arc::new(Message::tool(
                "c1",
                serde_json::to_string(&ToolResult::error("failing", "boom")).unwrap(),
            )),
            Arc::new(Message::assistant_with_tool_calls(
                "r2",
                vec![call_2.clone()],
            )),
            Arc::new(Message::tool(
                "c2",
                serde_json::to_string(&ToolResult::success("echo", json!({"ok": true}))).unwrap(),
            )),
        ];
        let stats = derive_stats_from_messages(&messages);
        assert_eq!(stats.step, 2);
        assert_eq!(stats.total_tool_call_count, 2);
        assert_eq!(stats.step_tool_call_count, 1);
        assert_eq!(stats.last_tool_calls.len(), 1);
        assert_eq!(stats.last_tool_calls[0].id, call_2.id);
        assert_eq!(stats.last_tool_calls[0].name, call_2.name);
        assert_eq!(stats.last_text, "r2");
        assert_eq!(stats.consecutive_errors, 0);
    }

    #[test]
    fn derives_stats_with_current_response() {
        let prior_messages = vec![Arc::new(Message::user("u1"))];
        let response = StreamResult {
            text: "r1".to_string(),
            tool_calls: vec![ToolCall::new("c1", "echo", json!({}))],
            usage: None,
        };

        let stats = derive_stats_from_messages_with_response(&prior_messages, &response);
        assert_eq!(stats.step, 1);
        assert_eq!(stats.step_tool_call_count, 1);
        assert_eq!(stats.total_tool_call_count, 1);
        assert_eq!(stats.last_text, "r1");
        assert_eq!(stats.last_tool_calls.len(), 1);
        assert_eq!(stats.last_tool_calls[0].id, "c1");
    }
}
