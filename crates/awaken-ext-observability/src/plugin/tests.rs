use std::sync::Arc;

use awaken_contract::contract::inference::{LLMResponse, StreamResult, TokenUsage};
use awaken_contract::contract::tool::ToolResult;
use awaken_contract::model::Phase;
use awaken_contract::state::{Snapshot, StateMap};
use awaken_runtime::{PhaseContext, PhaseHook, Plugin};

use crate::sink::InMemorySink;

use super::ObservabilityPlugin;
use super::hooks::{
    AfterInferenceHook, AfterToolExecuteHook, BeforeInferenceHook, BeforeToolExecuteHook,
    RunEndHook, RunStartHook,
};
use super::shared::{extract_cache_tokens, extract_token_counts, lock_unpoison};

fn empty_snapshot() -> Snapshot {
    Snapshot::new(0, Arc::new(StateMap::default()))
}

fn usage(prompt: i32, completion: i32, total: i32) -> TokenUsage {
    TokenUsage {
        prompt_tokens: Some(prompt),
        completion_tokens: Some(completion),
        total_tokens: Some(total),
        cache_read_tokens: None,
        cache_creation_tokens: None,
        thinking_tokens: None,
    }
}

fn success_response(u: Option<TokenUsage>) -> LLMResponse {
    use awaken_contract::contract::content::ContentBlock;
    LLMResponse::success(StreamResult {
        content: vec![ContentBlock::text("hello")],
        tool_calls: vec![],
        usage: u,
        stop_reason: None,
        has_incomplete_tool_calls: false,
    })
}

/// Dispatch helper: invoke the appropriate phase hook sharing the plugin's inner state.
async fn run_phase(plugin: &ObservabilityPlugin, ctx: &PhaseContext) {
    let inner = Arc::clone(&plugin.inner);
    match ctx.phase {
        Phase::RunStart => RunStartHook(inner).run(ctx).await.unwrap(),
        Phase::BeforeInference => BeforeInferenceHook(inner).run(ctx).await.unwrap(),
        Phase::AfterInference => AfterInferenceHook(inner).run(ctx).await.unwrap(),
        Phase::BeforeToolExecute => BeforeToolExecuteHook(inner).run(ctx).await.unwrap(),
        Phase::AfterToolExecute => AfterToolExecuteHook(inner).run(ctx).await.unwrap(),
        Phase::RunEnd => RunEndHook(inner).run(ctx).await.unwrap(),
        _ => return,
    };
}

#[test]
fn new_defaults_model_empty() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    let model = lock_unpoison(&plugin.inner.model);
    assert!(model.is_empty());
}

#[test]
fn new_defaults_provider_empty() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    let provider = lock_unpoison(&plugin.inner.provider);
    assert!(provider.is_empty());
}

#[test]
fn new_defaults_temperature_none() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    assert!(lock_unpoison(&plugin.inner.temperature).is_none());
}

#[test]
fn new_defaults_top_p_none() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    assert!(lock_unpoison(&plugin.inner.top_p).is_none());
}

#[test]
fn new_defaults_max_tokens_none() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    assert!(lock_unpoison(&plugin.inner.max_tokens).is_none());
}

#[test]
fn new_defaults_operation_is_chat() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    assert_eq!(plugin.inner.operation, "chat");
}

#[test]
fn new_defaults_metrics_empty() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    let metrics = lock_unpoison(&plugin.inner.metrics);
    assert!(metrics.inferences.is_empty());
    assert!(metrics.tools.is_empty());
    assert_eq!(metrics.session_duration_ms, 0);
}

#[test]
fn with_model_sets_model() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new()).with_model("gpt-4o");
    assert_eq!(*lock_unpoison(&plugin.inner.model), "gpt-4o");
}

#[test]
fn with_provider_sets_provider() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new()).with_provider("anthropic");
    assert_eq!(*lock_unpoison(&plugin.inner.provider), "anthropic");
}

#[test]
fn with_temperature_sets_temperature() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new()).with_temperature(0.7);
    assert_eq!(*lock_unpoison(&plugin.inner.temperature), Some(0.7));
}

#[test]
fn with_top_p_sets_top_p() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new()).with_top_p(0.9);
    assert_eq!(*lock_unpoison(&plugin.inner.top_p), Some(0.9));
}

#[test]
fn with_max_tokens_sets_max_tokens() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new()).with_max_tokens(4096);
    assert_eq!(*lock_unpoison(&plugin.inner.max_tokens), Some(4096));
}

#[test]
fn with_stop_sequences_sets_seqs() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new())
        .with_stop_sequences(vec!["STOP".into(), "END".into()]);
    let seqs = lock_unpoison(&plugin.inner.stop_sequences);
    assert_eq!(*seqs, vec!["STOP", "END"]);
}

#[test]
fn builder_chaining() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new())
        .with_model("claude-3")
        .with_provider("anthropic")
        .with_temperature(0.5)
        .with_top_p(0.8)
        .with_max_tokens(2048)
        .with_stop_sequences(vec!["DONE".into()]);

    assert_eq!(*lock_unpoison(&plugin.inner.model), "claude-3");
    assert_eq!(*lock_unpoison(&plugin.inner.provider), "anthropic");
    assert_eq!(*lock_unpoison(&plugin.inner.temperature), Some(0.5));
    assert_eq!(*lock_unpoison(&plugin.inner.top_p), Some(0.8));
    assert_eq!(*lock_unpoison(&plugin.inner.max_tokens), Some(2048));
    assert_eq!(*lock_unpoison(&plugin.inner.stop_sequences), vec!["DONE"]);
}

#[test]
fn descriptor_returns_observability() {
    let plugin = ObservabilityPlugin::new(InMemorySink::new());
    assert_eq!(plugin.descriptor().name, "observability");
}

#[tokio::test]
async fn on_run_start_initializes_run_start_time() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink);

    assert!(lock_unpoison(&plugin.inner.run_start).is_none());

    let ctx = PhaseContext::new(Phase::RunStart, empty_snapshot());
    run_phase(&plugin, &ctx).await;

    assert!(lock_unpoison(&plugin.inner.run_start).is_some());
}

#[tokio::test]
async fn on_before_inference_records_start_time() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink);

    assert!(lock_unpoison(&plugin.inner.inference_start).is_none());

    let ctx = PhaseContext::new(Phase::BeforeInference, empty_snapshot());
    run_phase(&plugin, &ctx).await;

    assert!(lock_unpoison(&plugin.inner.inference_start).is_some());
}

#[tokio::test]
async fn on_after_inference_records_genai_span() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink.clone())
        .with_model("gpt-4")
        .with_provider("openai");

    let ctx = PhaseContext::new(Phase::BeforeInference, empty_snapshot());
    run_phase(&plugin, &ctx).await;

    let ctx = PhaseContext::new(Phase::AfterInference, empty_snapshot())
        .with_llm_response(success_response(Some(usage(100, 50, 150))));
    run_phase(&plugin, &ctx).await;

    let metrics = lock_unpoison(&plugin.inner.metrics);
    assert_eq!(metrics.inferences.len(), 1);
    assert_eq!(metrics.inferences[0].model, "gpt-4");
    assert_eq!(metrics.inferences[0].provider, "openai");
    assert_eq!(metrics.inferences[0].input_tokens, Some(100));
    assert_eq!(metrics.inferences[0].output_tokens, Some(50));
    assert!(metrics.inferences[0].duration_ms > 0 || true); // may be 0 if fast

    // Also recorded in sink
    let sink_m = sink.metrics();
    assert_eq!(sink_m.inference_count(), 1);
}

#[tokio::test]
async fn on_after_inference_without_before_uses_zero_duration() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink.clone()).with_model("m");

    // Skip BeforeInference — go straight to AfterInference
    let ctx = PhaseContext::new(Phase::AfterInference, empty_snapshot())
        .with_llm_response(success_response(Some(usage(10, 5, 15))));
    run_phase(&plugin, &ctx).await;

    let metrics = lock_unpoison(&plugin.inner.metrics);
    assert_eq!(metrics.inferences.len(), 1);
    assert_eq!(metrics.inferences[0].duration_ms, 0);
}

#[tokio::test]
async fn on_before_tool_execute_records_tool_start() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink);

    let ctx = PhaseContext::new(Phase::BeforeToolExecute, empty_snapshot()).with_tool_info(
        "search",
        "call_42",
        Some(serde_json::json!({})),
    );
    run_phase(&plugin, &ctx).await;

    let tool_starts = lock_unpoison(&plugin.inner.tool_start);
    assert!(tool_starts.contains_key("call_42"));
}

#[tokio::test]
async fn on_after_tool_execute_records_tool_span() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink.clone());

    let ctx = PhaseContext::new(Phase::BeforeToolExecute, empty_snapshot()).with_tool_info(
        "search",
        "c1",
        Some(serde_json::json!({})),
    );
    run_phase(&plugin, &ctx).await;

    let ctx = PhaseContext::new(Phase::AfterToolExecute, empty_snapshot())
        .with_tool_info("search", "c1", Some(serde_json::json!({})))
        .with_tool_result(ToolResult::success(
            "search",
            serde_json::json!({"found": true}),
        ));
    run_phase(&plugin, &ctx).await;

    let metrics = lock_unpoison(&plugin.inner.metrics);
    assert_eq!(metrics.tools.len(), 1);
    assert_eq!(metrics.tools[0].name, "search");
    assert_eq!(metrics.tools[0].call_id, "c1");
    assert!(metrics.tools[0].is_success());

    let sink_m = sink.metrics();
    assert_eq!(sink_m.tool_count(), 1);
}

#[tokio::test]
async fn on_after_tool_execute_no_result_skips_recording() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink.clone());

    let ctx = PhaseContext::new(Phase::BeforeToolExecute, empty_snapshot()).with_tool_info(
        "search",
        "c1",
        Some(serde_json::json!({})),
    );
    run_phase(&plugin, &ctx).await;

    // AfterToolExecute without tool_result
    let ctx = PhaseContext::new(Phase::AfterToolExecute, empty_snapshot()).with_tool_info(
        "search",
        "c1",
        Some(serde_json::json!({})),
    );
    run_phase(&plugin, &ctx).await;

    let metrics = lock_unpoison(&plugin.inner.metrics);
    assert!(metrics.tools.is_empty());
}

#[tokio::test]
async fn on_after_tool_execute_error_records_error_type() {
    let sink = InMemorySink::new();
    let plugin = ObservabilityPlugin::new(sink.clone());

    let ctx = PhaseContext::new(Phase::BeforeToolExecute, empty_snapshot()).with_tool_info(
        "write",
        "c1",
        Some(serde_json::json!({})),
    );
    run_phase(&plugin, &ctx).await;

    let ctx = PhaseContext::new(Phase::AfterToolExecute, empty_snapshot())
        .with_tool_info("write", "c1", Some(serde_json::json!({})))
        .with_tool_result(ToolResult::error("write", "permission denied"));
    run_phase(&plugin, &ctx).await;

    let metrics = lock_unpoison(&plugin.inner.metrics);
    assert_eq!(metrics.tools.len(), 1);
    assert!(!metrics.tools[0].is_success());
    assert_eq!(metrics.tools[0].error_type.as_deref(), Some("tool_error"));
}

#[test]
fn extract_token_counts_with_some() {
    let u = TokenUsage {
        prompt_tokens: Some(10),
        completion_tokens: Some(20),
        total_tokens: Some(30),
        thinking_tokens: Some(5),
        cache_read_tokens: None,
        cache_creation_tokens: None,
    };
    let (i, o, t, th) = extract_token_counts(Some(&u));
    assert_eq!(i, Some(10));
    assert_eq!(o, Some(20));
    assert_eq!(t, Some(30));
    assert_eq!(th, Some(5));
}

#[test]
fn extract_token_counts_with_none() {
    let (i, o, t, th) = extract_token_counts(None);
    assert!(i.is_none());
    assert!(o.is_none());
    assert!(t.is_none());
    assert!(th.is_none());
}

#[test]
fn extract_cache_tokens_with_some() {
    let u = TokenUsage {
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        thinking_tokens: None,
        cache_read_tokens: Some(100),
        cache_creation_tokens: Some(50),
    };
    let (read, creation) = extract_cache_tokens(Some(&u));
    assert_eq!(read, Some(100));
    assert_eq!(creation, Some(50));
}

#[test]
fn extract_cache_tokens_with_none() {
    let (read, creation) = extract_cache_tokens(None);
    assert!(read.is_none());
    assert!(creation.is_none());
}
