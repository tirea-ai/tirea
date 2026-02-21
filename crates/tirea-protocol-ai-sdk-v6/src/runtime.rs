//! Runtime wiring for AI SDK requests.
//!
//! Applies AI SDK-specific extensions to a [`ResolvedRun`], currently
//! interaction-response replay plugin wiring.

use std::sync::Arc;
use tirea_agent_loop::runtime::loop_runner::ResolvedRun;
use tirea_extension_interaction::InteractionPlugin;

use crate::AiSdkV6RunRequest;

/// Apply AI SDK-specific extensions to a [`ResolvedRun`].
pub fn apply_ai_sdk_extensions(resolved: &mut ResolvedRun, request: &AiSdkV6RunRequest) {
    let interaction_plugin =
        InteractionPlugin::from_interaction_responses(request.interaction_responses());
    if interaction_plugin.is_active() {
        resolved.config.plugins.push(Arc::new(interaction_plugin));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AiSdkV6RunRequest;
    use serde_json::json;
    use std::collections::HashMap;
    use tirea_agent_loop::runtime::loop_runner::AgentConfig;
    use tirea_contract::RunConfig;

    fn empty_resolved() -> ResolvedRun {
        ResolvedRun {
            config: AgentConfig::default(),
            tools: HashMap::new(),
            run_config: RunConfig::new(),
        }
    }

    #[test]
    fn no_interaction_response_does_not_install_plugin() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t1",
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .expect("request should deserialize");
        let mut resolved = empty_resolved();
        apply_ai_sdk_extensions(&mut resolved, &req);
        assert!(resolved.config.plugins.is_empty());
    }

    #[test]
    fn interaction_response_installs_plugin() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t2",
            "messages": [{
                "role": "assistant",
                "parts": [{
                    "type": "tool-approval-response",
                    "approvalId": "fc_1",
                    "approved": true
                }]
            }]
        }))
        .expect("request should deserialize");
        let mut resolved = empty_resolved();
        apply_ai_sdk_extensions(&mut resolved, &req);
        assert_eq!(resolved.config.plugins.len(), 1);
    }
}
