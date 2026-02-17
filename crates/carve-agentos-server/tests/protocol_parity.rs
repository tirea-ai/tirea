use async_trait::async_trait;
use carve_agent::contracts::plugin::AgentPlugin;
use carve_agent::contracts::runtime::phase::Phase;
use carve_agent::contracts::runtime::phase::StepContext;
use carve_agent::contracts::runtime::{AgentEvent, RunRequest};
use carve_agent::orchestrator::{AgentOs, AgentOsBuilder};
use carve_agent::orchestrator::AgentDefinition;
use carve_protocol_ag_ui::{AGUIMessage, AgUiInputAdapter, RunAgentRequest};
use carve_protocol_ag_ui_runtime::build_agui_extensions;
use carve_protocol_ai_sdk_v6::{AiSdkV6InputAdapter, AiSdkV6RunRequest};
use carve_protocol_contract::ProtocolInputAdapter;
use carve_thread_store_adapters::MemoryStore;
use futures::StreamExt;
use std::sync::Arc;

struct SkipInferencePlugin;

#[async_trait]
impl AgentPlugin for SkipInferencePlugin {
    fn id(&self) -> &str {
        "skip_inference_parity"
    }

    async fn on_phase(
        &self,
        phase: Phase,
        step: &mut StepContext<'_>,
        _ctx: &carve_agent::prelude::AgentState,
    ) {
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
    }
}

fn make_os() -> AgentOs {
    let def = AgentDefinition {
        id: "test".to_string(),
        plugins: vec![Arc::new(SkipInferencePlugin)],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_agent("test", def)
        .with_agent_state_store(Arc::new(MemoryStore::new()))
        .build()
        .unwrap()
}

fn collect_kinds(events: &[AgentEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            AgentEvent::RunStart { .. } => "RunStart",
            AgentEvent::RunFinish { .. } => "RunFinish",
            AgentEvent::TextDelta { .. } => "TextDelta",
            AgentEvent::ToolCallStart { .. } => "ToolCallStart",
            AgentEvent::ToolCallDone { .. } => "ToolCallDone",
            AgentEvent::ToolCallReady { .. } => "ToolCallReady",
            AgentEvent::ToolCallDelta { .. } => "ToolCallDelta",
            AgentEvent::Pending { .. } => "Pending",
            AgentEvent::InteractionResolved { .. } => "InteractionResolved",
            AgentEvent::Error { .. } => "Error",
            AgentEvent::StateDelta { .. } => "StateDelta",
            AgentEvent::StateSnapshot { .. } => "StateSnapshot",
            AgentEvent::MessagesSnapshot { .. } => "MessagesSnapshot",
            AgentEvent::ActivityDelta { .. } => "ActivityDelta",
            AgentEvent::ActivitySnapshot { .. } => "ActivitySnapshot",
            AgentEvent::StepStart { .. } => "StepStart",
            AgentEvent::StepEnd => "StepEnd",
            AgentEvent::InferenceComplete { .. } => "InferenceComplete",
            AgentEvent::InteractionRequested { .. } => "InteractionRequested",
        })
        .collect()
}

fn normalize(run: RunRequest) -> RunRequest {
    RunRequest {
        parent_run_id: None,
        resource_id: None,
        state: None,
        ..run
    }
}

#[test]
fn agui_and_ai_sdk_inputs_map_to_equivalent_run_requests() {
    let agent_id = "test".to_string();
    let agui = RunAgentRequest::new("thread_parity", "run_parity")
        .with_message(AGUIMessage::user("hello parity"));
    let aisdk = AiSdkV6RunRequest {
        thread_id: "thread_parity".to_string(),
        input: "hello parity".to_string(),
        run_id: Some("run_parity".to_string()),
    };

    let agui_run = normalize(AgUiInputAdapter::to_run_request(agent_id.clone(), agui));
    let aisdk_run = normalize(AiSdkV6InputAdapter::to_run_request(agent_id, aisdk));

    assert_eq!(agui_run.agent_id, aisdk_run.agent_id);
    assert_eq!(agui_run.thread_id, aisdk_run.thread_id);
    assert_eq!(agui_run.run_id, aisdk_run.run_id);
    assert_eq!(agui_run.messages.len(), 1);
    assert_eq!(aisdk_run.messages.len(), 1);
    assert_eq!(agui_run.messages[0].role, aisdk_run.messages[0].role);
    assert_eq!(agui_run.messages[0].content, aisdk_run.messages[0].content);
}

#[tokio::test]
async fn agui_and_ai_sdk_have_equivalent_runtime_event_shape() {
    let os = make_os();

    let agui_req = RunAgentRequest::new("thread_parity_stream", "run_parity_stream")
        .with_message(AGUIMessage::user("hello parity"));
    let agui_extensions = build_agui_extensions(&agui_req);
    let agui_run_req = AgUiInputAdapter::to_run_request("test".to_string(), agui_req);
    let agui_run = os
        .run_stream_with_extensions(agui_run_req, agui_extensions)
        .await
        .unwrap();
    let agui_events: Vec<AgentEvent> = agui_run.events.collect().await;

    let aisdk_run_req = AiSdkV6InputAdapter::to_run_request(
        "test".to_string(),
        AiSdkV6RunRequest {
            thread_id: "thread_parity_stream".to_string(),
            input: "hello parity".to_string(),
            run_id: Some("run_parity_stream".to_string()),
        },
    );
    let aisdk_run = os.run_stream(aisdk_run_req).await.unwrap();
    let aisdk_events: Vec<AgentEvent> = aisdk_run.events.collect().await;

    assert_eq!(
        collect_kinds(&agui_events),
        collect_kinds(&aisdk_events),
        "AG-UI and AI-SDK should produce equivalent runtime event shape for the same semantic input"
    );
}
