//! LLM telemetry plugin aligned with OpenTelemetry GenAI Semantic Conventions.
//!
//! Captures per-inference and per-tool metrics via the Phase system,
//! forwarding them to a pluggable [`MetricsSink`].

use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use async_trait::async_trait;
use genai::chat::Usage;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

// =============================================================================
// Span types (OTel GenAI aligned)
// =============================================================================

/// A single LLM inference span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenAISpan {
    /// Model identifier (e.g. "gpt-4o-mini"). OTel: `gen_ai.request.model`.
    pub model: String,
    /// Provider name (e.g. "openai"). OTel: `gen_ai.provider.name`.
    pub provider: String,
    /// Operation name (e.g. "chat"). OTel: `gen_ai.operation.name`.
    pub operation: String,
    /// Response model (may differ from request model). OTel: `gen_ai.response.model`.
    pub response_model: Option<String>,
    /// Response ID. OTel: `gen_ai.response.id`.
    pub response_id: Option<String>,
    /// Finish reasons. OTel: `gen_ai.response.finish_reasons`.
    pub finish_reasons: Vec<String>,
    /// Error type if the inference failed. OTel: `error.type`.
    pub error_type: Option<String>,
    /// Input (prompt) tokens. OTel: `gen_ai.usage.input_tokens`.
    pub input_tokens: Option<i32>,
    /// Output (completion) tokens. OTel: `gen_ai.usage.output_tokens`.
    pub output_tokens: Option<i32>,
    /// Total tokens (non-OTel convenience).
    pub total_tokens: Option<i32>,
    /// Cache-read input tokens. OTel: `gen_ai.usage.cache_read.input_tokens`.
    pub cache_read_input_tokens: Option<i32>,
    /// Cache-creation input tokens. OTel: `gen_ai.usage.cache_creation.input_tokens`.
    pub cache_creation_input_tokens: Option<i32>,
    /// Wall-clock duration in milliseconds. OTel: `gen_ai.client.operation.duration`.
    pub duration_ms: u64,
}

/// A single tool execution span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpan {
    /// Tool name. OTel: `gen_ai.tool.name`.
    pub name: String,
    /// Operation name (always "execute_tool"). OTel: `gen_ai.operation.name`.
    pub operation: String,
    /// Tool call ID. OTel: `gen_ai.tool.call.id`.
    pub call_id: String,
    /// Error type when the tool failed. OTel: `error.type`.
    pub error_type: Option<String>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

impl ToolSpan {
    /// Whether the tool execution succeeded (no error).
    pub fn is_success(&self) -> bool {
        self.error_type.is_none()
    }
}

/// Aggregated metrics for an agent session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentMetrics {
    /// All inference spans.
    pub inferences: Vec<GenAISpan>,
    /// All tool spans.
    pub tools: Vec<ToolSpan>,
    /// Total session duration in milliseconds.
    pub session_duration_ms: u64,
}

impl AgentMetrics {
    /// Total input tokens across all inferences.
    pub fn total_input_tokens(&self) -> i32 {
        self.inferences
            .iter()
            .filter_map(|s| s.input_tokens)
            .sum()
    }

    /// Total output tokens across all inferences.
    pub fn total_output_tokens(&self) -> i32 {
        self.inferences
            .iter()
            .filter_map(|s| s.output_tokens)
            .sum()
    }

    /// Total tokens across all inferences.
    pub fn total_tokens(&self) -> i32 {
        self.inferences
            .iter()
            .filter_map(|s| s.total_tokens)
            .sum()
    }

    /// Number of inferences.
    pub fn inference_count(&self) -> usize {
        self.inferences.len()
    }

    /// Number of tool executions.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Number of failed tool executions.
    pub fn tool_failures(&self) -> usize {
        self.tools.iter().filter(|t| !t.is_success()).count()
    }
}

// =============================================================================
// MetricsSink trait
// =============================================================================

/// Trait for consuming telemetry data.
///
/// Implementations can send data to OTel collectors, log files, etc.
pub trait MetricsSink: Send + Sync {
    /// Called when an inference completes.
    fn on_inference(&self, span: &GenAISpan);

    /// Called when a tool execution completes.
    fn on_tool(&self, span: &ToolSpan);

    /// Called when a session ends with aggregated metrics.
    fn on_session_end(&self, metrics: &AgentMetrics);
}

// =============================================================================
// InMemorySink
// =============================================================================

/// In-memory sink for testing and inspection.
#[derive(Debug, Clone, Default)]
pub struct InMemorySink {
    inner: Arc<Mutex<AgentMetrics>>,
}

impl InMemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a snapshot of the current metrics.
    pub fn metrics(&self) -> AgentMetrics {
        self.inner.lock().unwrap().clone()
    }
}

impl MetricsSink for InMemorySink {
    fn on_inference(&self, span: &GenAISpan) {
        self.inner.lock().unwrap().inferences.push(span.clone());
    }

    fn on_tool(&self, span: &ToolSpan) {
        self.inner.lock().unwrap().tools.push(span.clone());
    }

    fn on_session_end(&self, metrics: &AgentMetrics) {
        let mut inner = self.inner.lock().unwrap();
        inner.session_duration_ms = metrics.session_duration_ms;
    }
}

// =============================================================================
// LLMMetryPlugin
// =============================================================================

/// Plugin that captures LLM and tool telemetry.
///
/// Attach an [`InMemorySink`] (or custom sink) to collect metrics:
///
/// ```ignore
/// let sink = InMemorySink::new();
/// let plugin = LLMMetryPlugin::new(sink.clone())
///     .with_model("gpt-4o-mini")
///     .with_provider("openai");
/// let config = AgentDefinition::new("gpt-4o-mini")
///     .with_plugin(Arc::new(plugin));
/// // ... run agent ...
/// let metrics = sink.metrics();
/// let _total = metrics.total_tokens();
/// ```
pub struct LLMMetryPlugin {
    sink: Arc<dyn MetricsSink>,
    session_start: Mutex<Option<Instant>>,
    metrics: Mutex<AgentMetrics>,
    /// Inference timing: set at BeforeInference, consumed at AfterInference.
    inference_start: Mutex<Option<Instant>>,
    /// Tool timing: set at BeforeToolExecute, consumed at AfterToolExecute.
    tool_start: Mutex<Option<Instant>>,
    /// Model name captured from AgentDefinition (set externally or from data).
    model: Mutex<String>,
    /// Provider name (e.g. "openai", "anthropic"). OTel: `gen_ai.provider.name`.
    provider: Mutex<String>,
    /// Operation name (defaults to "chat").
    operation: String,
    /// Tracing span for the current inference (created at BeforeInference, closed at AfterInference).
    inference_tracing_span: Mutex<Option<tracing::Span>>,
    /// Tracing span for the current tool execution (created at BeforeToolExecute, closed at AfterToolExecute).
    tool_tracing_span: Mutex<Option<tracing::Span>>,
}

impl LLMMetryPlugin {
    pub fn new(sink: impl MetricsSink + 'static) -> Self {
        Self {
            sink: Arc::new(sink),
            session_start: Mutex::new(None),
            metrics: Mutex::new(AgentMetrics::default()),
            inference_start: Mutex::new(None),
            tool_start: Mutex::new(None),
            model: Mutex::new(String::new()),
            provider: Mutex::new(String::new()),
            operation: "chat".to_string(),
            inference_tracing_span: Mutex::new(None),
            tool_tracing_span: Mutex::new(None),
        }
    }

    pub fn with_model(self, model: impl Into<String>) -> Self {
        *self.model.lock().unwrap() = model.into();
        self
    }

    pub fn with_provider(self, provider: impl Into<String>) -> Self {
        *self.provider.lock().unwrap() = provider.into();
        self
    }
}

#[async_trait]
impl AgentPlugin for LLMMetryPlugin {
    fn id(&self) -> &str {
        "llmmetry"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        match phase {
            Phase::SessionStart => {
                *self.session_start.lock().unwrap() = Some(Instant::now());
            }
            Phase::BeforeInference => {
                *self.inference_start.lock().unwrap() = Some(Instant::now());
                let model = self.model.lock().unwrap().clone();
                let provider = self.provider.lock().unwrap().clone();
                let span_name = format!("{} {}", self.operation, model);
                let span = tracing::info_span!("gen_ai",
                    "otel.name" = %span_name,
                    "otel.kind" = "client",
                    "otel.status_code" = tracing::field::Empty,
                    "otel.status_description" = tracing::field::Empty,
                    "gen_ai.provider.name" = %provider,
                    "gen_ai.operation.name" = %self.operation,
                    "gen_ai.request.model" = %model,
                    "gen_ai.response.model" = tracing::field::Empty,
                    "gen_ai.response.id" = tracing::field::Empty,
                    "gen_ai.usage.input_tokens" = tracing::field::Empty,
                    "gen_ai.usage.output_tokens" = tracing::field::Empty,
                    "gen_ai.usage.cache_read.input_tokens" = tracing::field::Empty,
                    "gen_ai.usage.cache_creation.input_tokens" = tracing::field::Empty,
                    "gen_ai.client.operation.duration_ms" = tracing::field::Empty,
                    "error.type" = tracing::field::Empty,
                );
                step.tracing_span = Some(span.clone());
                *self.inference_tracing_span.lock().unwrap() = Some(span);
            }
            Phase::AfterInference => {
                // Clear the step's reference so the span can fully close
                step.tracing_span.take();

                let duration_ms = self
                    .inference_start
                    .lock()
                    .unwrap()
                    .take()
                    .map(|s| s.elapsed().as_millis() as u64)
                    .unwrap_or(0);

                let usage = step.response.as_ref().and_then(|r| r.usage.as_ref());
                let (input_tokens, output_tokens, total_tokens) = extract_token_counts(usage);
                let (cache_read_input_tokens, cache_creation_input_tokens) =
                    extract_cache_tokens(usage);

                let model = self.model.lock().unwrap().clone();
                let provider = self.provider.lock().unwrap().clone();
                let span = GenAISpan {
                    model,
                    provider,
                    operation: self.operation.clone(),
                    response_model: None,
                    response_id: None,
                    finish_reasons: Vec::new(),
                    error_type: None,
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cache_read_input_tokens,
                    cache_creation_input_tokens,
                    duration_ms,
                };

                // Record fields onto the tracing span and drop it (closing the OTel span).
                if let Some(tracing_span) = self.inference_tracing_span.lock().unwrap().take() {
                    tracing_span.record("gen_ai.usage.input_tokens", span.input_tokens.unwrap_or(0));
                    tracing_span.record("gen_ai.usage.output_tokens", span.output_tokens.unwrap_or(0));
                    if let Some(v) = span.cache_read_input_tokens {
                        tracing_span.record("gen_ai.usage.cache_read.input_tokens", v);
                    }
                    if let Some(v) = span.cache_creation_input_tokens {
                        tracing_span.record("gen_ai.usage.cache_creation.input_tokens", v);
                    }
                    if let Some(ref v) = span.response_model {
                        tracing_span.record("gen_ai.response.model", v.as_str());
                    }
                    if let Some(ref v) = span.response_id {
                        tracing_span.record("gen_ai.response.id", v.as_str());
                    }
                    if let Some(ref v) = span.error_type {
                        tracing_span.record("error.type", v.as_str());
                        tracing_span.record("otel.status_code", "ERROR");
                        tracing_span.record("otel.status_description", v.as_str());
                    }
                    tracing_span.record("gen_ai.client.operation.duration_ms", span.duration_ms);
                    drop(tracing_span);
                }

                self.sink.on_inference(&span);
                self.metrics.lock().unwrap().inferences.push(span);
            }
            Phase::BeforeToolExecute => {
                *self.tool_start.lock().unwrap() = Some(Instant::now());
                let tool_name = step
                    .tool
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                let call_id = step
                    .tool
                    .as_ref()
                    .map(|t| t.id.clone())
                    .unwrap_or_default();
                let provider = self.provider.lock().unwrap().clone();
                let span_name = format!("execute_tool {}", tool_name);
                let span = tracing::info_span!("gen_ai",
                    "otel.name" = %span_name,
                    "otel.kind" = "internal",
                    "otel.status_code" = tracing::field::Empty,
                    "otel.status_description" = tracing::field::Empty,
                    "gen_ai.provider.name" = %provider,
                    "gen_ai.operation.name" = "execute_tool",
                    "gen_ai.tool.name" = %tool_name,
                    "gen_ai.tool.call.id" = %call_id,
                    "error.type" = tracing::field::Empty,
                    "tool.duration_ms" = tracing::field::Empty,
                );
                step.tracing_span = Some(span.clone());
                *self.tool_tracing_span.lock().unwrap() = Some(span);
            }
            Phase::AfterToolExecute => {
                // Clear the step's reference so the span can fully close
                step.tracing_span.take();

                let duration_ms = self
                    .tool_start
                    .lock()
                    .unwrap()
                    .take()
                    .map(|s| s.elapsed().as_millis() as u64)
                    .unwrap_or(0);

                let (name, call_id, error_type) = if let Some(ref tc) = step.tool {
                    let ok = tc
                        .result
                        .as_ref()
                        .map(|r| r.status == crate::traits::tool::ToolStatus::Success)
                        .unwrap_or(false);
                    let err = if !ok {
                        tc.result
                            .as_ref()
                            .and_then(|r| r.message.clone())
                    } else {
                        None
                    };
                    (tc.name.clone(), tc.id.clone(), err)
                } else {
                    ("unknown".to_string(), String::new(), None)
                };

                let span = ToolSpan {
                    name,
                    operation: "execute_tool".to_string(),
                    call_id,
                    error_type,
                    duration_ms,
                };

                if let Some(tracing_span) = self.tool_tracing_span.lock().unwrap().take() {
                    if let Some(ref v) = span.error_type {
                        tracing_span.record("error.type", v.as_str());
                        tracing_span.record("otel.status_code", "ERROR");
                        tracing_span.record("otel.status_description", v.as_str());
                    }
                    tracing_span.record("tool.duration_ms", span.duration_ms);
                    drop(tracing_span);
                }

                self.sink.on_tool(&span);
                self.metrics.lock().unwrap().tools.push(span);
            }
            Phase::SessionEnd => {
                let session_duration_ms = self
                    .session_start
                    .lock()
                    .unwrap()
                    .take()
                    .map(|s| s.elapsed().as_millis() as u64)
                    .unwrap_or(0);

                let mut metrics = self.metrics.lock().unwrap().clone();
                metrics.session_duration_ms = session_duration_ms;
                self.sink.on_session_end(&metrics);
            }
            _ => {}
        }
    }
}

fn extract_token_counts(usage: Option<&Usage>) -> (Option<i32>, Option<i32>, Option<i32>) {
    match usage {
        Some(u) => (u.prompt_tokens, u.completion_tokens, u.total_tokens),
        None => (None, None, None),
    }
}

fn extract_cache_tokens(usage: Option<&Usage>) -> (Option<i32>, Option<i32>) {
    match usage.and_then(|u| u.prompt_tokens_details.as_ref()) {
        Some(d) => (d.cached_tokens, d.cache_creation_tokens),
        None => (None, None),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::ToolContext as PhaseToolContext;
    use crate::session::Session;
    use crate::stream::StreamResult;
    use crate::traits::tool::ToolResult;
    use crate::types::ToolCall;
    use genai::chat::PromptTokensDetails;
    use serde_json::json;

    fn mock_session() -> Session {
        Session::new("test")
    }

    fn usage(prompt: i32, completion: i32, total: i32) -> Usage {
        Usage {
            prompt_tokens: Some(prompt),
            prompt_tokens_details: None,
            completion_tokens: Some(completion),
            completion_tokens_details: None,
            total_tokens: Some(total),
        }
    }

    fn usage_with_cache(prompt: i32, completion: i32, total: i32, cached: i32) -> Usage {
        Usage {
            prompt_tokens: Some(prompt),
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: Some(cached),
                cache_creation_tokens: None,
                audio_tokens: None,
            }),
            completion_tokens: Some(completion),
            completion_tokens_details: None,
            total_tokens: Some(total),
        }
    }

    fn make_span(model: &str, provider: &str) -> GenAISpan {
        GenAISpan {
            model: model.into(),
            provider: provider.into(),
            operation: "chat".into(),
            response_model: None,
            response_id: None,
            finish_reasons: Vec::new(),
            error_type: None,
            input_tokens: Some(10),
            output_tokens: Some(20),
            total_tokens: Some(30),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            duration_ms: 100,
        }
    }

    fn make_tool_span(name: &str, call_id: &str) -> ToolSpan {
        ToolSpan {
            name: name.into(),
            operation: "execute_tool".into(),
            call_id: call_id.into(),
            error_type: None,
            duration_ms: 10,
        }
    }

    // ---- ToolSpan::is_success ----

    #[test]
    fn test_tool_span_is_success() {
        let span = make_tool_span("search", "c1");
        assert!(span.is_success());

        let span = ToolSpan {
            error_type: Some("permission denied".into()),
            ..make_tool_span("write", "c2")
        };
        assert!(!span.is_success());
    }

    // ---- AgentMetrics ----

    #[test]
    fn test_agent_metrics_defaults() {
        let m = AgentMetrics::default();
        assert_eq!(m.total_input_tokens(), 0);
        assert_eq!(m.total_output_tokens(), 0);
        assert_eq!(m.total_tokens(), 0);
        assert_eq!(m.inference_count(), 0);
        assert_eq!(m.tool_count(), 0);
        assert_eq!(m.tool_failures(), 0);
    }

    #[test]
    fn test_agent_metrics_aggregation() {
        let m = AgentMetrics {
            inferences: vec![
                make_span("m", "openai"),
                GenAISpan {
                    input_tokens: Some(5),
                    output_tokens: None,
                    total_tokens: Some(8),
                    duration_ms: 50,
                    ..make_span("m", "openai")
                },
            ],
            tools: vec![
                make_tool_span("a", "c1"),
                ToolSpan {
                    error_type: Some("permission denied".into()),
                    ..make_tool_span("b", "c2")
                },
            ],
            session_duration_ms: 500,
        };
        assert_eq!(m.total_input_tokens(), 15);
        assert_eq!(m.total_output_tokens(), 20);
        assert_eq!(m.total_tokens(), 38);
        assert_eq!(m.inference_count(), 2);
        assert_eq!(m.tool_count(), 2);
        assert_eq!(m.tool_failures(), 1);
    }

    // ---- InMemorySink ----

    #[test]
    fn test_in_memory_sink_collects() {
        let sink = InMemorySink::new();
        sink.on_inference(&make_span("test", "openai"));
        sink.on_tool(&make_tool_span("t", "c1"));
        let m = sink.metrics();
        assert_eq!(m.inference_count(), 1);
        assert_eq!(m.tool_count(), 1);
    }

    #[test]
    fn test_in_memory_sink_session_end() {
        let sink = InMemorySink::new();
        let metrics = AgentMetrics {
            session_duration_ms: 999,
            ..Default::default()
        };
        sink.on_session_end(&metrics);
        assert_eq!(sink.metrics().session_duration_ms, 999);
    }

    // ---- LLMMetryPlugin ----

    #[tokio::test]
    async fn test_plugin_captures_inference() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone())
            .with_model("gpt-4")
            .with_provider("openai");

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        plugin.on_phase(Phase::BeforeInference, &mut step).await;

        step.response = Some(StreamResult {
            text: "hello".into(),
            tool_calls: vec![],
            usage: Some(usage(100, 50, 150)),
        });

        plugin.on_phase(Phase::AfterInference, &mut step).await;

        let m = sink.metrics();
        assert_eq!(m.inference_count(), 1);
        assert_eq!(m.total_input_tokens(), 100);
        assert_eq!(m.total_output_tokens(), 50);
        assert_eq!(m.inferences[0].model, "gpt-4");
        assert_eq!(m.inferences[0].provider, "openai");
        assert_eq!(m.inferences[0].operation, "chat");
        assert!(m.inferences[0].cache_read_input_tokens.is_none());
    }

    #[tokio::test]
    async fn test_plugin_captures_inference_with_cache() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone())
            .with_model("gpt-4")
            .with_provider("openai");

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        plugin.on_phase(Phase::BeforeInference, &mut step).await;

        step.response = Some(StreamResult {
            text: "hello".into(),
            tool_calls: vec![],
            usage: Some(usage_with_cache(100, 50, 150, 30)),
        });

        plugin.on_phase(Phase::AfterInference, &mut step).await;

        let m = sink.metrics();
        let span = &m.inferences[0];
        assert_eq!(span.cache_read_input_tokens, Some(30));
        assert!(span.cache_creation_input_tokens.is_none());
    }

    #[tokio::test]
    async fn test_plugin_captures_tool() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone());

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("c1", "search", json!({}));
        step.tool = Some(PhaseToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step)
            .await;

        step.tool.as_mut().unwrap().result =
            Some(ToolResult::success("search", json!({"found": true})));

        plugin
            .on_phase(Phase::AfterToolExecute, &mut step)
            .await;

        let m = sink.metrics();
        assert_eq!(m.tool_count(), 1);
        assert!(m.tools[0].is_success());
        assert_eq!(m.tools[0].name, "search");
        assert_eq!(m.tools[0].call_id, "c1");
        assert_eq!(m.tools[0].operation, "execute_tool");
        assert!(m.tools[0].error_type.is_none());
    }

    #[tokio::test]
    async fn test_plugin_captures_tool_failure() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone());

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("c1", "write", json!({}));
        step.tool = Some(PhaseToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step)
            .await;

        step.tool.as_mut().unwrap().result =
            Some(ToolResult::error("write", "permission denied"));

        plugin
            .on_phase(Phase::AfterToolExecute, &mut step)
            .await;

        let m = sink.metrics();
        assert!(!m.tools[0].is_success());
        assert_eq!(m.tools[0].error_type.as_deref(), Some("permission denied"));
    }

    #[tokio::test]
    async fn test_plugin_session_lifecycle() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone());

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        plugin.on_phase(Phase::SessionStart, &mut step).await;

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        plugin.on_phase(Phase::SessionEnd, &mut step).await;

        let m = sink.metrics();
        assert!(m.session_duration_ms >= 10);
    }

    #[tokio::test]
    async fn test_plugin_no_usage() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone()).with_model("m");

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        plugin.on_phase(Phase::BeforeInference, &mut step).await;
        step.response = Some(StreamResult {
            text: "hi".into(),
            tool_calls: vec![],
            usage: None,
        });
        plugin.on_phase(Phase::AfterInference, &mut step).await;

        let m = sink.metrics();
        assert_eq!(m.inference_count(), 1);
        assert!(m.inferences[0].input_tokens.is_none());
        assert!(m.inferences[0].cache_read_input_tokens.is_none());
    }

    #[tokio::test]
    async fn test_plugin_multiple_rounds() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone()).with_model("m");

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        for i in 0..3 {
            plugin.on_phase(Phase::BeforeInference, &mut step).await;
            step.response = Some(StreamResult {
                text: format!("r{i}"),
                tool_calls: vec![],
                usage: Some(usage(10 * (i + 1), 5 * (i + 1), 15 * (i + 1))),
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;
        }

        let m = sink.metrics();
        assert_eq!(m.inference_count(), 3);
        assert_eq!(m.total_input_tokens(), 60); // 10+20+30
        assert_eq!(m.total_output_tokens(), 30); // 5+10+15
    }

    #[test]
    fn test_genai_span_serialization() {
        let span = make_span("gpt-4", "openai");
        let json = serde_json::to_value(&span).unwrap();
        assert_eq!(json["model"], "gpt-4");
        assert_eq!(json["input_tokens"], 10);
        assert_eq!(json["provider"], "openai");
        assert_eq!(json["operation"], "chat");
    }

    #[test]
    fn test_tool_span_serialization() {
        let span = make_tool_span("search", "c1");
        let json = serde_json::to_value(&span).unwrap();
        assert_eq!(json["name"], "search");
        assert_eq!(json["call_id"], "c1");
        assert_eq!(json["operation"], "execute_tool");
    }

    #[test]
    fn test_agent_metrics_serialization() {
        let m = AgentMetrics::default();
        let json = serde_json::to_string(&m).unwrap();
        let m2: AgentMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(m2.session_duration_ms, 0);
    }

    #[test]
    fn test_extract_token_counts_some() {
        let u = usage(10, 20, 30);
        let (i, o, t) = extract_token_counts(Some(&u));
        assert_eq!(i, Some(10));
        assert_eq!(o, Some(20));
        assert_eq!(t, Some(30));
    }

    #[test]
    fn test_extract_token_counts_none() {
        let (i, o, t) = extract_token_counts(None);
        assert!(i.is_none());
        assert!(o.is_none());
        assert!(t.is_none());
    }

    #[test]
    fn test_extract_cache_tokens() {
        let u = usage_with_cache(100, 50, 150, 30);
        let (read, creation) = extract_cache_tokens(Some(&u));
        assert_eq!(read, Some(30));
        assert!(creation.is_none());
    }

    #[test]
    fn test_extract_cache_tokens_none() {
        assert_eq!(extract_cache_tokens(None), (None, None));
        let u = usage(10, 20, 30);
        assert_eq!(extract_cache_tokens(Some(&u)), (None, None));
    }

    // ---- Tracing span capture tests ----

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::registry::LookupSpan;

    #[derive(Debug, Clone)]
    struct CapturedSpan {
        name: String,
        was_closed: bool,
    }

    struct SpanCaptureLayer {
        captured: Arc<Mutex<Vec<CapturedSpan>>>,
    }

    impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> tracing_subscriber::Layer<S>
        for SpanCaptureLayer
    {
        fn on_new_span(
            &self,
            _attrs: &tracing::span::Attributes<'_>,
            id: &tracing::span::Id,
            ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if let Some(span_ref) = ctx.span(id) {
                self.captured.lock().unwrap().push(CapturedSpan {
                    name: span_ref.name().to_string(),
                    was_closed: false,
                });
            }
        }

        fn on_close(&self, id: tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
            if let Some(span_ref) = ctx.span(&id) {
                let name = span_ref.name().to_string();
                let mut captured = self.captured.lock().unwrap();
                if let Some(entry) = captured.iter_mut().find(|c| c.name == name) {
                    entry.was_closed = true;
                }
            }
        }
    }

    #[tokio::test]
    async fn test_tracing_span_inference() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedSpan>::new()));
        let layer = SpanCaptureLayer {
            captured: captured.clone(),
        };
        let subscriber = tracing_subscriber::registry::Registry::default().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone())
            .with_model("test-model")
            .with_provider("test-provider");

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        plugin.on_phase(Phase::BeforeInference, &mut step).await;
        step.response = Some(StreamResult {
            text: "hi".into(),
            tool_calls: vec![],
            usage: Some(usage(10, 20, 30)),
        });
        plugin.on_phase(Phase::AfterInference, &mut step).await;

        let spans = captured.lock().unwrap();
        let inference_span = spans.iter().find(|s| s.name == "gen_ai");
        assert!(inference_span.is_some(), "expected gen_ai span (inference)");
        assert!(inference_span.unwrap().was_closed, "span should be closed");
    }

    #[tokio::test]
    async fn test_tracing_span_tool() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedSpan>::new()));
        let layer = SpanCaptureLayer {
            captured: captured.clone(),
        };
        let subscriber = tracing_subscriber::registry::Registry::default().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone());

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        let call = ToolCall::new("c1", "search", json!({}));
        step.tool = Some(PhaseToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step)
            .await;
        step.tool.as_mut().unwrap().result =
            Some(ToolResult::success("search", json!({"found": true})));
        plugin
            .on_phase(Phase::AfterToolExecute, &mut step)
            .await;

        let spans = captured.lock().unwrap();
        let tool_span = spans.iter().find(|s| s.name == "gen_ai");
        assert!(tool_span.is_some(), "expected gen_ai span (tool)");
        assert!(tool_span.unwrap().was_closed, "span should be closed");
    }

    #[test]
    fn test_plugin_id() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink);
        assert_eq!(plugin.id(), "llmmetry");
    }

    // ---- OTel export compatibility tests ----

    mod otel_export {
        use super::*;
        use opentelemetry::trace::TracerProvider as _;
        use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
        use tracing_opentelemetry::OpenTelemetryLayer;
        use tracing_subscriber::layer::SubscriberExt;

        fn setup_otel_test() -> (
            tracing::subscriber::DefaultGuard,
            InMemorySpanExporter,
            SdkTracerProvider,
        ) {
            let exporter = InMemorySpanExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            let tracer = provider.tracer("test");
            let otel_layer = OpenTelemetryLayer::new(tracer);
            let subscriber = tracing_subscriber::registry::Registry::default().with(otel_layer);
            let guard = tracing::subscriber::set_default(subscriber);
            (guard, exporter, provider)
        }

        fn find_attribute<'a>(
            span: &'a SpanData,
            key: &str,
        ) -> Option<&'a opentelemetry::Value> {
            span.attributes
                .iter()
                .find(|kv| kv.key.as_str() == key)
                .map(|kv| &kv.value)
        }

        #[tokio::test]
        async fn test_otel_export_inference_span() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("test-model")
                .with_provider("test-provider");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            plugin.on_phase(Phase::BeforeInference, &mut step).await;
            step.response = Some(StreamResult {
                text: "hello".into(),
                tool_calls: vec![],
                usage: Some(usage(100, 50, 150)),
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span in OTel export");

            assert_eq!(
                find_attribute(span, "gen_ai.provider.name").unwrap().as_str(),
                "test-provider"
            );
            assert_eq!(
                find_attribute(span, "gen_ai.operation.name").unwrap().as_str(),
                "chat"
            );
            assert_eq!(
                find_attribute(span, "gen_ai.request.model").unwrap().as_str(),
                "test-model"
            );
            assert_eq!(
                find_attribute(span, "gen_ai.usage.input_tokens"),
                Some(&opentelemetry::Value::I64(100))
            );
            assert_eq!(
                find_attribute(span, "gen_ai.usage.output_tokens"),
                Some(&opentelemetry::Value::I64(50))
            );
            assert!(
                find_attribute(span, "gen_ai.client.operation.duration_ms").is_some(),
                "expected duration_ms attribute"
            );
        }

        #[tokio::test]
        async fn test_otel_export_parent_child_propagation() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("test-model")
                .with_provider("test-provider");

            // Create and enter a parent span
            let parent = tracing::info_span!("parent_operation");
            let _parent_guard = parent.enter();

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            plugin.on_phase(Phase::BeforeInference, &mut step).await;

            // Verify tracing_span is set on step context
            assert!(
                step.tracing_span.is_some(),
                "BeforeInference should set tracing_span on StepContext"
            );

            step.response = Some(StreamResult {
                text: "hello".into(),
                tool_calls: vec![],
                usage: Some(usage(100, 50, 150)),
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;

            drop(_parent_guard);
            drop(parent);

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();

            let parent_span = exported
                .iter()
                .find(|s| s.name == "parent_operation")
                .expect("expected parent_operation span");
            let child_span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span");

            // The child span should share the same trace_id as the parent
            assert_eq!(
                child_span.span_context.trace_id(),
                parent_span.span_context.trace_id(),
                "chat span should share parent's trace_id"
            );
            // The child span's parent_span_id should be the parent's span_id
            assert_eq!(
                child_span.parent_span_id,
                parent_span.span_context.span_id(),
                "chat span should have parent's span_id as parent_span_id"
            );
        }

        #[tokio::test]
        async fn test_otel_export_tool_parent_child_propagation() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink);

            let parent = tracing::info_span!("parent_tool_op");
            let _parent_guard = parent.enter();

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            let call = ToolCall::new("tc1", "search", json!({}));
            step.tool = Some(PhaseToolContext::new(&call));

            plugin
                .on_phase(Phase::BeforeToolExecute, &mut step)
                .await;

            assert!(
                step.tracing_span.is_some(),
                "BeforeToolExecute should set tracing_span on StepContext"
            );

            step.tool.as_mut().unwrap().result =
                Some(ToolResult::success("search", json!({"found": true})));
            plugin
                .on_phase(Phase::AfterToolExecute, &mut step)
                .await;

            drop(_parent_guard);
            drop(parent);

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();

            let parent_span = exported
                .iter()
                .find(|s| s.name == "parent_tool_op")
                .expect("expected parent_tool_op span");
            let child_span = exported
                .iter()
                .find(|s| s.name.starts_with("execute_tool "))
                .expect("expected execute_tool span");

            assert_eq!(
                child_span.span_context.trace_id(),
                parent_span.span_context.trace_id(),
                "execute_tool span should share parent's trace_id"
            );
            assert_eq!(
                child_span.parent_span_id,
                parent_span.span_context.span_id(),
                "execute_tool span should have parent's span_id as parent_span_id"
            );
        }

        #[tokio::test]
        async fn test_otel_export_no_parent_context() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("m")
                .with_provider("p");

            // No parent span entered — should be a root span
            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            plugin.on_phase(Phase::BeforeInference, &mut step).await;
            step.response = Some(StreamResult {
                text: "hi".into(),
                tool_calls: vec![],
                usage: None,
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();

            let span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span");

            // Root span should have invalid parent_span_id
            assert_eq!(
                span.parent_span_id,
                opentelemetry::trace::SpanId::INVALID,
                "root span should have no parent"
            );
        }

        #[tokio::test]
        async fn test_otel_export_tracing_span_cleared_after_phases() {
            let (_guard, _exporter, _provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("m")
                .with_provider("p");

            let session = mock_session();

            // Test inference: tracing_span should be None after AfterInference
            {
                let mut step = StepContext::new(&session, vec![]);
                plugin.on_phase(Phase::BeforeInference, &mut step).await;
                assert!(step.tracing_span.is_some());
                step.response = Some(StreamResult {
                    text: "hi".into(),
                    tool_calls: vec![],
                    usage: None,
                });
                plugin.on_phase(Phase::AfterInference, &mut step).await;
                assert!(
                    step.tracing_span.is_none(),
                    "AfterInference should clear tracing_span"
                );
            }

            // Test tool: tracing_span should be None after AfterToolExecute
            {
                let mut step = StepContext::new(&session, vec![]);
                let call = ToolCall::new("c1", "test", json!({}));
                step.tool = Some(PhaseToolContext::new(&call));
                plugin
                    .on_phase(Phase::BeforeToolExecute, &mut step)
                    .await;
                assert!(step.tracing_span.is_some());
                step.tool.as_mut().unwrap().result =
                    Some(ToolResult::success("test", json!({})));
                plugin
                    .on_phase(Phase::AfterToolExecute, &mut step)
                    .await;
                assert!(
                    step.tracing_span.is_none(),
                    "AfterToolExecute should clear tracing_span"
                );
            }
        }

        #[tokio::test]
        async fn test_otel_export_inference_and_tool_are_siblings() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("m")
                .with_provider("p");

            let parent = tracing::info_span!("agent_step");
            let _parent_guard = parent.enter();

            let session = mock_session();

            // Inference phase
            {
                let mut step = StepContext::new(&session, vec![]);
                plugin.on_phase(Phase::BeforeInference, &mut step).await;
                step.response = Some(StreamResult {
                    text: "calling tool".into(),
                    tool_calls: vec![ToolCall::new("c1", "search", json!({}))],
                    usage: Some(usage(10, 5, 15)),
                });
                plugin.on_phase(Phase::AfterInference, &mut step).await;
            }

            // Tool phase
            {
                let mut step = StepContext::new(&session, vec![]);
                let call = ToolCall::new("c1", "search", json!({}));
                step.tool = Some(PhaseToolContext::new(&call));
                plugin
                    .on_phase(Phase::BeforeToolExecute, &mut step)
                    .await;
                step.tool.as_mut().unwrap().result =
                    Some(ToolResult::success("search", json!({})));
                plugin
                    .on_phase(Phase::AfterToolExecute, &mut step)
                    .await;
            }

            drop(_parent_guard);
            drop(parent);

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();

            let parent_span = exported
                .iter()
                .find(|s| s.name == "agent_step")
                .expect("expected agent_step span");
            let inference_span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span");
            let tool_span = exported
                .iter()
                .find(|s| s.name.starts_with("execute_tool "))
                .expect("expected execute_tool span");

            // Both should share the same trace
            assert_eq!(
                inference_span.span_context.trace_id(),
                parent_span.span_context.trace_id(),
            );
            assert_eq!(
                tool_span.span_context.trace_id(),
                parent_span.span_context.trace_id(),
            );

            // Both should be children of the parent (siblings, not nested)
            assert_eq!(
                inference_span.parent_span_id,
                parent_span.span_context.span_id(),
                "inference span should be child of agent_step"
            );
            assert_eq!(
                tool_span.parent_span_id,
                parent_span.span_context.span_id(),
                "tool span should be child of agent_step"
            );

            // They should be distinct spans
            assert_ne!(
                inference_span.span_context.span_id(),
                tool_span.span_context.span_id(),
                "inference and tool should be distinct sibling spans"
            );
        }

        #[tokio::test]
        async fn test_otel_export_tool_span() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink);

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            let call = ToolCall::new("tc1", "search", json!({}));
            step.tool = Some(PhaseToolContext::new(&call));

            plugin
                .on_phase(Phase::BeforeToolExecute, &mut step)
                .await;
            step.tool.as_mut().unwrap().result =
                Some(ToolResult::success("search", json!({"found": true})));
            plugin
                .on_phase(Phase::AfterToolExecute, &mut step)
                .await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("execute_tool "))
                .expect("expected execute_tool span in OTel export");

            assert_eq!(
                find_attribute(span, "gen_ai.tool.name").unwrap().as_str(),
                "search"
            );
            assert_eq!(
                find_attribute(span, "gen_ai.tool.call.id").unwrap().as_str(),
                "tc1"
            );
            assert_eq!(
                find_attribute(span, "gen_ai.operation.name").unwrap().as_str(),
                "execute_tool"
            );
        }

        // ==================================================================
        // OTel GenAI Semantic Conventions compliance tests
        // ==================================================================

        #[tokio::test]
        async fn test_otel_semconv_inference_span_name_format() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("gpt-4o")
                .with_provider("openai");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            plugin.on_phase(Phase::BeforeInference, &mut step).await;
            step.response = Some(StreamResult {
                text: "hi".into(),
                tool_calls: vec![],
                usage: None,
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span");

            // Spec: span name = "{gen_ai.operation.name} {gen_ai.request.model}"
            assert_eq!(
                span.name.as_ref(),
                "chat gpt-4o",
                "span name should follow OTel GenAI format: '{{operation}} {{model}}'"
            );
        }

        #[tokio::test]
        async fn test_otel_semconv_tool_span_name_format() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink).with_provider("openai");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            let call = ToolCall::new("tc1", "web_search", json!({}));
            step.tool = Some(PhaseToolContext::new(&call));

            plugin
                .on_phase(Phase::BeforeToolExecute, &mut step)
                .await;
            step.tool.as_mut().unwrap().result =
                Some(ToolResult::success("web_search", json!({})));
            plugin
                .on_phase(Phase::AfterToolExecute, &mut step)
                .await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("execute_tool "))
                .expect("expected execute_tool span");

            // Spec: span name = "execute_tool {gen_ai.tool.name}"
            assert_eq!(
                span.name.as_ref(),
                "execute_tool web_search",
                "span name should follow OTel GenAI format: 'execute_tool {{tool_name}}'"
            );
        }

        #[tokio::test]
        async fn test_otel_semconv_inference_span_kind_client() {
            use opentelemetry::trace::SpanKind;

            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("m")
                .with_provider("p");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            plugin.on_phase(Phase::BeforeInference, &mut step).await;
            step.response = Some(StreamResult {
                text: "hi".into(),
                tool_calls: vec![],
                usage: None,
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span");

            // Spec: chat spans should be SpanKind::Client
            assert_eq!(
                span.span_kind,
                SpanKind::Client,
                "inference span should have SpanKind::Client per OTel GenAI spec"
            );
        }

        #[tokio::test]
        async fn test_otel_semconv_tool_span_kind_internal() {
            use opentelemetry::trace::SpanKind;

            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink).with_provider("p");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            let call = ToolCall::new("tc1", "search", json!({}));
            step.tool = Some(PhaseToolContext::new(&call));

            plugin
                .on_phase(Phase::BeforeToolExecute, &mut step)
                .await;
            step.tool.as_mut().unwrap().result =
                Some(ToolResult::success("search", json!({})));
            plugin
                .on_phase(Phase::AfterToolExecute, &mut step)
                .await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("execute_tool "))
                .expect("expected execute_tool span");

            // Spec: tool spans should be SpanKind::Internal
            assert_eq!(
                span.span_kind,
                SpanKind::Internal,
                "tool span should have SpanKind::Internal per OTel GenAI spec"
            );
        }

        #[tokio::test]
        async fn test_otel_semconv_tool_span_has_provider() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_provider("anthropic");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            let call = ToolCall::new("tc1", "search", json!({}));
            step.tool = Some(PhaseToolContext::new(&call));

            plugin
                .on_phase(Phase::BeforeToolExecute, &mut step)
                .await;
            step.tool.as_mut().unwrap().result =
                Some(ToolResult::success("search", json!({})));
            plugin
                .on_phase(Phase::AfterToolExecute, &mut step)
                .await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("execute_tool "))
                .expect("expected execute_tool span");

            // Spec: gen_ai.provider.name is required on all GenAI spans
            assert_eq!(
                find_attribute(span, "gen_ai.provider.name").unwrap().as_str(),
                "anthropic",
                "tool span must include gen_ai.provider.name per OTel GenAI spec"
            );
        }

        #[tokio::test]
        async fn test_otel_semconv_error_sets_status_code() {
            use opentelemetry::trace::Status;

            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink).with_provider("p");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            let call = ToolCall::new("tc1", "write", json!({}));
            step.tool = Some(PhaseToolContext::new(&call));

            plugin
                .on_phase(Phase::BeforeToolExecute, &mut step)
                .await;
            step.tool.as_mut().unwrap().result =
                Some(ToolResult::error("write", "permission denied"));
            plugin
                .on_phase(Phase::AfterToolExecute, &mut step)
                .await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("execute_tool "))
                .expect("expected execute_tool span");

            // Spec: error.type should be set
            assert_eq!(
                find_attribute(span, "error.type").unwrap().as_str(),
                "permission denied"
            );

            // Spec: OTel status should be Error on failure
            assert!(
                matches!(span.status, Status::Error { .. }),
                "failed tool span should have OTel Status::Error, got {:?}",
                span.status
            );
        }

        #[tokio::test]
        async fn test_otel_semconv_success_no_error_status() {
            use opentelemetry::trace::Status;

            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("m")
                .with_provider("p");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            plugin.on_phase(Phase::BeforeInference, &mut step).await;
            step.response = Some(StreamResult {
                text: "ok".into(),
                tool_calls: vec![],
                usage: Some(usage(10, 5, 15)),
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span");

            // Spec: successful spans should NOT have Error status
            assert!(
                !matches!(span.status, Status::Error { .. }),
                "successful span should not have Error status"
            );
            assert!(
                find_attribute(span, "error.type").is_none(),
                "successful span should not have error.type attribute"
            );
        }

        #[tokio::test]
        async fn test_otel_semconv_required_attributes_present() {
            let (_guard, exporter, provider) = setup_otel_test();

            let sink = InMemorySink::new();
            let plugin = LLMMetryPlugin::new(sink)
                .with_model("claude-3")
                .with_provider("anthropic");

            let session = mock_session();
            let mut step = StepContext::new(&session, vec![]);

            plugin.on_phase(Phase::BeforeInference, &mut step).await;
            step.response = Some(StreamResult {
                text: "hi".into(),
                tool_calls: vec![],
                usage: Some(usage(100, 50, 150)),
            });
            plugin.on_phase(Phase::AfterInference, &mut step).await;

            let _ = provider.force_flush();
            let exported = exporter.get_finished_spans().unwrap();
            let span = exported
                .iter()
                .find(|s| s.name.starts_with("chat "))
                .expect("expected chat span");

            // Required: gen_ai.operation.name
            assert_eq!(
                find_attribute(span, "gen_ai.operation.name").unwrap().as_str(),
                "chat"
            );
            // Required: gen_ai.provider.name
            assert_eq!(
                find_attribute(span, "gen_ai.provider.name").unwrap().as_str(),
                "anthropic"
            );
            // Conditionally required: gen_ai.request.model
            assert_eq!(
                find_attribute(span, "gen_ai.request.model").unwrap().as_str(),
                "claude-3"
            );
            // Recommended: gen_ai.usage.input_tokens
            assert_eq!(
                find_attribute(span, "gen_ai.usage.input_tokens"),
                Some(&opentelemetry::Value::I64(100))
            );
            // Recommended: gen_ai.usage.output_tokens
            assert_eq!(
                find_attribute(span, "gen_ai.usage.output_tokens"),
                Some(&opentelemetry::Value::I64(50))
            );
        }
    }
}
