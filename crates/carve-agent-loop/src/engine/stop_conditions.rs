//! Stop condition contracts and evaluator.

pub use carve_agent_contract::agent::stop::{
    ConsecutiveErrors, ContentMatch, LoopDetection, MaxRounds, StopCheckContext, StopCondition,
    StopConditionSpec, StopOnTool, Timeout, TokenBudget,
};
pub use carve_agent_contract::runtime::StopReason;
use std::sync::Arc;

/// Check all conditions in order, returning the first match.
pub fn check_stop_conditions(
    conditions: &[Arc<dyn StopCondition>],
    ctx: &StopCheckContext,
) -> Option<StopReason> {
    for condition in conditions {
        if let Some(reason) = condition.check(ctx) {
            return Some(reason);
        }
    }
    None
}
