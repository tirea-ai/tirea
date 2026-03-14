#![allow(missing_docs)]

use async_trait::async_trait;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use futures::StreamExt;
use genai::chat::{
    ChatRequest, ChatResponse, ChatStreamEvent, MessageContent, StreamChunk, StreamEnd, ToolChunk,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tirea_agentos::composition::{AgentDefinition, AgentDefinitionSpec, AgentOsBuilder};
use tirea_agentos::runtime::loop_runner::{tool_map_from_arc, LlmEventStream, LlmExecutor};
use tirea_agentos::runtime::AgentOs;
use tirea_contract::runtime::tool_call::{Tool, ToolDescriptor, ToolError, ToolResult};
use tirea_contract::storage::RunOrigin;
use tirea_contract::{Message, RunRequest, ToolCallContext};
use tirea_store_adapters::MemoryStore;

// ---------------------------------------------------------------------------
// ScriptedMockLlm
// ---------------------------------------------------------------------------

enum ScriptedStep {
    TextReply(String),
    ToolCalls(Vec<(String, String, Value)>), // (call_id, tool_name, args)
}

struct ScriptedMockLlm {
    script: Vec<ScriptedStep>,
    cursor: AtomicUsize,
}

impl ScriptedMockLlm {
    fn new(script: Vec<ScriptedStep>) -> Self {
        Self {
            script,
            cursor: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmExecutor for ScriptedMockLlm {
    async fn exec_chat_response(
        &self,
        _model: &str,
        _chat_req: ChatRequest,
        _options: Option<&genai::chat::ChatOptions>,
    ) -> genai::Result<ChatResponse> {
        unimplemented!("stream-only mock")
    }

    async fn exec_chat_stream_events(
        &self,
        _model: &str,
        _chat_req: ChatRequest,
        _options: Option<&genai::chat::ChatOptions>,
    ) -> genai::Result<LlmEventStream> {
        let idx = self.cursor.fetch_add(1, Ordering::SeqCst);
        let step = self.script.get(idx);

        let events: Vec<genai::Result<ChatStreamEvent>> = match step {
            Some(ScriptedStep::TextReply(text)) => {
                vec![
                    Ok(ChatStreamEvent::Start),
                    Ok(ChatStreamEvent::Chunk(StreamChunk {
                        content: text.clone(),
                    })),
                    Ok(ChatStreamEvent::End(StreamEnd::default())),
                ]
            }
            Some(ScriptedStep::ToolCalls(calls)) => {
                let tool_calls: Vec<genai::chat::ToolCall> = calls
                    .iter()
                    .map(|(call_id, fn_name, args)| genai::chat::ToolCall {
                        call_id: call_id.clone(),
                        fn_name: fn_name.clone(),
                        fn_arguments: Value::String(args.to_string()),
                        thought_signatures: None,
                    })
                    .collect();

                let mut events = vec![Ok(ChatStreamEvent::Start)];
                for tc in &tool_calls {
                    events.push(Ok(ChatStreamEvent::ToolCallChunk(ToolChunk {
                        tool_call: tc.clone(),
                    })));
                }
                events.push(Ok(ChatStreamEvent::End(StreamEnd {
                    captured_content: Some(MessageContent::from_tool_calls(tool_calls)),
                    ..Default::default()
                })));
                events
            }
            None => {
                // Past end of script: emit a plain text reply to end the loop.
                vec![
                    Ok(ChatStreamEvent::Start),
                    Ok(ChatStreamEvent::Chunk(StreamChunk {
                        content: "done".to_string(),
                    })),
                    Ok(ChatStreamEvent::End(StreamEnd::default())),
                ]
            }
        };

        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &'static str {
        "scripted_mock"
    }
}

// ---------------------------------------------------------------------------
// EchoTool
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("echo", "Echo", "Echoes input").with_parameters(json!({
            "type": "object",
            "properties": {
                "input": {"type": "string"}
            },
            "required": ["input"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success("echo", json!({"echoed": args})))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_bench_os() -> AgentOs {
    let store = Arc::new(MemoryStore::new());
    AgentOsBuilder::new()
        .with_tools(tool_map_from_arc(vec![Arc::new(EchoTool) as Arc<dyn Tool>]))
        .with_agent_spec(AgentDefinitionSpec::local(
            AgentDefinition::with_id("agent", "mock-model")
                .with_system_prompt("You are a benchmark agent.")
                .with_max_rounds(50),
        ))
        .with_agent_state_store(store)
        .build()
        .expect("failed to build benchmark AgentOs")
}

fn make_script(turns: usize) -> Vec<ScriptedStep> {
    let mut steps = Vec::with_capacity(turns + 1);
    for i in 0..turns {
        steps.push(ScriptedStep::ToolCalls(vec![(
            format!("call_{i}"),
            "echo".to_string(),
            json!({"input": format!("turn_{i}")}),
        )]));
    }
    steps.push(ScriptedStep::TextReply("benchmark complete".to_string()));
    steps
}

static THREAD_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_thread_id() -> String {
    let n = THREAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("bench-thread-{n}")
}

async fn run_to_completion(os: &AgentOs, llm: Arc<dyn LlmExecutor>, thread_id: String) -> usize {
    let request = RunRequest {
        agent_id: "agent".to_string(),
        thread_id: Some(thread_id),
        run_id: None,
        parent_run_id: None,
        parent_thread_id: None,
        resource_id: None,
        origin: RunOrigin::default(),
        state: None,
        messages: vec![Message::user("benchmark prompt")],
        initial_decisions: vec![],
        source_mailbox_entry_id: None,
    };

    let mut resolved = os.resolve(&request.agent_id).expect("resolve failed");
    resolved.agent = resolved.agent.with_llm_executor(llm);
    let prepared = os
        .prepare_run(request, resolved)
        .await
        .expect("prepare_run failed");
    let run = AgentOs::execute_prepared(prepared).expect("execute_prepared failed");
    let events: Vec<_> = run.events.collect().await;
    events.len()
}

// ---------------------------------------------------------------------------
// Benchmark Group 1: single_agent_lifecycle
// ---------------------------------------------------------------------------

fn bench_single_agent_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_agent_lifecycle");
    let os = build_bench_os();

    for turns in [1, 3, 5, 10] {
        group.throughput(Throughput::Elements(turns as u64));
        group.bench_with_input(BenchmarkId::from_parameter(turns), &turns, |b, &turns| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter(|| {
                    let os = &os;
                    let llm =
                        Arc::new(ScriptedMockLlm::new(make_script(turns))) as Arc<dyn LlmExecutor>;
                    let thread_id = unique_thread_id();
                    async move {
                        black_box(run_to_completion(os, llm, thread_id).await);
                    }
                });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark Group 2: concurrent_agents
// ---------------------------------------------------------------------------

fn bench_concurrent_agents(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_agents");
    let os = build_bench_os();

    for concurrency in [1, 2, 4, 8, 16, 32] {
        group.throughput(Throughput::Elements(concurrency as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            &concurrency,
            |b, &concurrency| {
                b.to_async(tokio::runtime::Runtime::new().unwrap())
                    .iter(|| {
                        let os = &os;
                        async move {
                            let handles: Vec<_> = (0..concurrency)
                                .map(|_| {
                                    let llm = Arc::new(ScriptedMockLlm::new(make_script(3)))
                                        as Arc<dyn LlmExecutor>;
                                    let thread_id = unique_thread_id();
                                    let os_ref = os.clone();
                                    tokio::spawn(async move {
                                        run_to_completion(&os_ref, llm, thread_id).await
                                    })
                                })
                                .collect();

                            let results = futures::future::join_all(handles).await;
                            black_box(
                                results
                                    .into_iter()
                                    .map(|r| r.expect("task panicked"))
                                    .sum::<usize>(),
                            );
                        }
                    });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark Group 3: memory_per_agent (wall-clock proxy)
// ---------------------------------------------------------------------------

fn bench_memory_per_agent(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_per_agent");
    let os = build_bench_os();

    for turns in [1, 5, 10] {
        group.throughput(Throughput::Elements(turns as u64));
        group.bench_with_input(BenchmarkId::from_parameter(turns), &turns, |b, &turns| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter(|| {
                    let os = &os;
                    let llm =
                        Arc::new(ScriptedMockLlm::new(make_script(turns))) as Arc<dyn LlmExecutor>;
                    let thread_id = unique_thread_id();
                    async move {
                        black_box(run_to_completion(os, llm, thread_id).await);
                    }
                });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion harness
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_single_agent_lifecycle,
    bench_concurrent_agents,
    bench_memory_per_agent,
);
criterion_main!(benches);
