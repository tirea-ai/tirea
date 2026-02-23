use async_trait::async_trait;
use futures::future::ready;
use std::sync::Arc;
use tirea_agentos::contracts::plugin::phase::BeforeInferenceContext;
use tirea_agentos::contracts::plugin::AgentPlugin;
use tirea_agentos::contracts::storage::AgentStateReader;
use tirea_agentos::contracts::RunRequest;
use tirea_agentos::orchestrator::{AgentDefinition, AgentOs, AgentOsBuilder};
use tirea_agentos_server::transport::pump_encoded_stream;
use tirea_contract::ProtocolInputAdapter;
use tirea_protocol_ai_sdk_v6::{AiSdkV6InputAdapter, AiSdkV6ProtocolEncoder, AiSdkV6RunRequest};
use tirea_store_adapters::MemoryStore;

struct TerminatePluginRequestedPlugin;

#[async_trait]
impl AgentPlugin for TerminatePluginRequestedPlugin {
    fn id(&self) -> &str {
        "terminate_plugin_requested_cross_crate_matrix"
    }

    async fn before_inference(&self, step: &mut BeforeInferenceContext<'_, '_>) {
        step.terminate_plugin_requested();
    }
}

fn make_os(store: Arc<MemoryStore>) -> AgentOs {
    let def = AgentDefinition {
        id: "test".to_string(),
        plugin_ids: vec!["terminate_plugin_requested_cross_crate_matrix".into()],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_registered_plugin(
            "terminate_plugin_requested_cross_crate_matrix",
            Arc::new(TerminatePluginRequestedPlugin),
        )
        .with_agent("test", def)
        .with_agent_state_store(store)
        .build()
        .expect("failed to build AgentOs")
}

#[tokio::test]
async fn cross_crate_integration_matrix_72() {
    let store = Arc::new(MemoryStore::new());
    let os = make_os(store.clone());

    let thread_cases = ["", " ", "cross-a", "cross-b", "session-42", "\t"];
    let run_cases = [None, Some("run-fixed"), Some("run-alt")];
    let input_cases = ["hello", "input-42", "{\"x\":1}", "line1\\nline2"];

    let mut executed = 0usize;

    for thread_id in thread_cases {
        for run_id in run_cases {
            for input in input_cases {
                let req = AiSdkV6RunRequest::from_thread_input(
                    thread_id,
                    input,
                    run_id.map(str::to_string),
                );
                let run_request: RunRequest =
                    AiSdkV6InputAdapter::to_run_request("test".to_string(), req);

                let run = os
                    .run_stream(run_request)
                    .await
                    .expect("run_stream should succeed");

                let resolved_thread_id = run.thread_id.clone();
                let resolved_run_id = run.run_id.clone();

                let mut encoded = Vec::new();
                let encoder = AiSdkV6ProtocolEncoder::new(
                    resolved_run_id.clone(),
                    Some(resolved_thread_id.clone()),
                );
                pump_encoded_stream(run.events, encoder, |event| {
                    encoded.push(
                        serde_json::to_value(event).expect("protocol event must be serializable"),
                    );
                    ready(Ok(()))
                })
                .await;

                assert!(
                    encoded
                        .iter()
                        .any(|e| e.get("type").and_then(|v| v.as_str()) == Some("start")),
                    "missing ai-sdk start event for thread={resolved_thread_id}, run={resolved_run_id}"
                );
                assert!(
                    encoded
                        .iter()
                        .any(|e| e.get("type").and_then(|v| v.as_str()) == Some("finish")),
                    "missing ai-sdk finish event for thread={resolved_thread_id}, run={resolved_run_id}"
                );

                if thread_id.trim().is_empty() {
                    assert!(
                        !resolved_thread_id.trim().is_empty(),
                        "auto-generated thread id must not be empty"
                    );
                } else {
                    assert_eq!(resolved_thread_id, thread_id);
                }

                assert!(
                    !resolved_run_id.trim().is_empty(),
                    "resolved run id must not be empty"
                );

                let saved = store
                    .load_agent_state(&resolved_thread_id)
                    .await
                    .expect("load should not fail")
                    .expect("thread must be persisted");
                assert!(
                    saved.messages.iter().any(|m| m.content == input),
                    "persisted thread should contain user input"
                );

                executed += 1;
            }
        }
    }

    assert_eq!(executed, 72, "cross-crate scenario count drifted");
}
