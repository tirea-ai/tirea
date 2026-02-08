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

/// Breakdown of input (prompt) token usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputTokensDetails {
    /// Tokens read from cache (Anthropic: `cache_read_input_tokens`).
    pub cached_tokens: Option<i32>,
    /// Tokens used to create cache entries (Anthropic: `cache_creation_input_tokens`).
    pub cache_creation_tokens: Option<i32>,
    /// Audio input tokens.
    pub audio_tokens: Option<i32>,
}

/// Breakdown of output (completion) token usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    /// Tokens spent on chain-of-thought reasoning.
    pub reasoning_tokens: Option<i32>,
    /// Audio output tokens.
    pub audio_tokens: Option<i32>,
    /// Accepted speculative/predicted tokens.
    pub accepted_prediction_tokens: Option<i32>,
    /// Rejected speculative/predicted tokens.
    pub rejected_prediction_tokens: Option<i32>,
}

/// A single LLM inference span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenAISpan {
    /// Model identifier (e.g. "gpt-4o-mini"). OTel: `gen_ai.request.model`.
    pub model: String,
    /// Provider/system (e.g. "openai"). OTel: `gen_ai.system`.
    pub system: String,
    /// Operation name (e.g. "chat"). OTel: `gen_ai.operation.name`.
    pub operation: String,
    /// Input (prompt) tokens. OTel: `gen_ai.usage.input_tokens`.
    pub input_tokens: Option<i32>,
    /// Output (completion) tokens. OTel: `gen_ai.usage.output_tokens`.
    pub output_tokens: Option<i32>,
    /// Total tokens.
    pub total_tokens: Option<i32>,
    /// Wall-clock duration in milliseconds. OTel: `gen_ai.client.operation.duration`.
    pub duration_ms: u64,
    /// Input token breakdown.
    pub input_tokens_details: Option<InputTokensDetails>,
    /// Output token breakdown.
    pub output_tokens_details: Option<OutputTokensDetails>,
}

/// A single tool execution span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpan {
    /// Tool name.
    pub name: String,
    /// Whether the tool succeeded.
    pub success: bool,
    /// Error type when `success` is false. OTel: `error.type`.
    pub error_type: Option<String>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
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
        self.tools.iter().filter(|t| !t.success).count()
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
///     .with_system("openai");
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
    /// Provider/system identifier (e.g. "openai", "anthropic").
    system: Mutex<String>,
    /// Operation name (defaults to "chat").
    operation: String,
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
            system: Mutex::new(String::new()),
            operation: "chat".to_string(),
        }
    }

    pub fn with_model(self, model: impl Into<String>) -> Self {
        *self.model.lock().unwrap() = model.into();
        self
    }

    pub fn with_system(self, system: impl Into<String>) -> Self {
        *self.system.lock().unwrap() = system.into();
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
            }
            Phase::AfterInference => {
                let duration_ms = self
                    .inference_start
                    .lock()
                    .unwrap()
                    .take()
                    .map(|s| s.elapsed().as_millis() as u64)
                    .unwrap_or(0);

                let usage = step.response.as_ref().and_then(|r| r.usage.as_ref());
                let (input_tokens, output_tokens, total_tokens) = extract_token_counts(usage);
                let input_tokens_details = extract_input_details(usage);
                let output_tokens_details = extract_output_details(usage);

                let model = self.model.lock().unwrap().clone();
                let system = self.system.lock().unwrap().clone();
                let span = GenAISpan {
                    model,
                    system,
                    operation: self.operation.clone(),
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    duration_ms,
                    input_tokens_details,
                    output_tokens_details,
                };

                tracing::info!(
                    gen_ai.system = %span.system,
                    gen_ai.request.model = %span.model,
                    gen_ai.operation.name = %span.operation,
                    gen_ai.usage.input_tokens = ?span.input_tokens,
                    gen_ai.usage.output_tokens = ?span.output_tokens,
                    gen_ai.client.operation.duration_ms = span.duration_ms,
                    "gen_ai.inference.complete"
                );

                self.sink.on_inference(&span);
                self.metrics.lock().unwrap().inferences.push(span);
            }
            Phase::BeforeToolExecute => {
                *self.tool_start.lock().unwrap() = Some(Instant::now());
            }
            Phase::AfterToolExecute => {
                let duration_ms = self
                    .tool_start
                    .lock()
                    .unwrap()
                    .take()
                    .map(|s| s.elapsed().as_millis() as u64)
                    .unwrap_or(0);

                let (name, success, error_type) = if let Some(ref tc) = step.tool {
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
                    (tc.name.clone(), ok, err)
                } else {
                    ("unknown".to_string(), false, None)
                };

                let span = ToolSpan {
                    name,
                    success,
                    error_type,
                    duration_ms,
                };

                tracing::info!(
                    tool.name = %span.name,
                    tool.success = span.success,
                    error.r#type = ?span.error_type,
                    tool.duration_ms = span.duration_ms,
                    "gen_ai.tool.complete"
                );

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

fn extract_input_details(usage: Option<&Usage>) -> Option<InputTokensDetails> {
    let d = usage?.prompt_tokens_details.as_ref()?;
    Some(InputTokensDetails {
        cached_tokens: d.cached_tokens,
        cache_creation_tokens: d.cache_creation_tokens,
        audio_tokens: d.audio_tokens,
    })
}

fn extract_output_details(usage: Option<&Usage>) -> Option<OutputTokensDetails> {
    let d = usage?.completion_tokens_details.as_ref()?;
    Some(OutputTokensDetails {
        reasoning_tokens: d.reasoning_tokens,
        audio_tokens: d.audio_tokens,
        accepted_prediction_tokens: d.accepted_prediction_tokens,
        rejected_prediction_tokens: d.rejected_prediction_tokens,
    })
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
    use genai::chat::{CompletionTokensDetails, PromptTokensDetails};
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

    fn usage_with_details(
        prompt: i32,
        completion: i32,
        total: i32,
        cached: i32,
        reasoning: i32,
    ) -> Usage {
        Usage {
            prompt_tokens: Some(prompt),
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: Some(cached),
                cache_creation_tokens: None,
                audio_tokens: None,
            }),
            completion_tokens: Some(completion),
            completion_tokens_details: Some(CompletionTokensDetails {
                reasoning_tokens: Some(reasoning),
                audio_tokens: None,
                accepted_prediction_tokens: None,
                rejected_prediction_tokens: None,
            }),
            total_tokens: Some(total),
        }
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
                GenAISpan {
                    model: "m".into(),
                    system: "openai".into(),
                    operation: "chat".into(),
                    input_tokens: Some(10),
                    output_tokens: Some(20),
                    total_tokens: Some(30),
                    duration_ms: 100,
                    input_tokens_details: None,
                    output_tokens_details: None,
                },
                GenAISpan {
                    model: "m".into(),
                    system: "openai".into(),
                    operation: "chat".into(),
                    input_tokens: Some(5),
                    output_tokens: None,
                    total_tokens: Some(8),
                    duration_ms: 50,
                    input_tokens_details: None,
                    output_tokens_details: None,
                },
            ],
            tools: vec![
                ToolSpan {
                    name: "a".into(),
                    success: true,
                    error_type: None,
                    duration_ms: 10,
                },
                ToolSpan {
                    name: "b".into(),
                    success: false,
                    error_type: Some("permission denied".into()),
                    duration_ms: 20,
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
        let span = GenAISpan {
            model: "test".into(),
            system: "openai".into(),
            operation: "chat".into(),
            input_tokens: Some(10),
            output_tokens: Some(20),
            total_tokens: Some(30),
            duration_ms: 100,
            input_tokens_details: None,
            output_tokens_details: None,
        };
        sink.on_inference(&span);
        sink.on_tool(&ToolSpan {
            name: "t".into(),
            success: true,
            error_type: None,
            duration_ms: 5,
        });
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
            .with_system("openai");

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
        assert_eq!(m.inferences[0].system, "openai");
        assert_eq!(m.inferences[0].operation, "chat");
    }

    #[tokio::test]
    async fn test_plugin_captures_inference_with_details() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink.clone())
            .with_model("gpt-4")
            .with_system("openai");

        let session = mock_session();
        let mut step = StepContext::new(&session, vec![]);

        plugin.on_phase(Phase::BeforeInference, &mut step).await;

        step.response = Some(StreamResult {
            text: "hello".into(),
            tool_calls: vec![],
            usage: Some(usage_with_details(100, 50, 150, 30, 10)),
        });

        plugin.on_phase(Phase::AfterInference, &mut step).await;

        let m = sink.metrics();
        let span = &m.inferences[0];
        let input_d = span.input_tokens_details.as_ref().unwrap();
        assert_eq!(input_d.cached_tokens, Some(30));
        assert!(input_d.cache_creation_tokens.is_none());
        let output_d = span.output_tokens_details.as_ref().unwrap();
        assert_eq!(output_d.reasoning_tokens, Some(10));
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
        assert!(m.tools[0].success);
        assert_eq!(m.tools[0].name, "search");
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
        assert!(!m.tools[0].success);
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
        assert!(m.inferences[0].input_tokens_details.is_none());
        assert!(m.inferences[0].output_tokens_details.is_none());
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
        let span = GenAISpan {
            model: "gpt-4".into(),
            system: "openai".into(),
            operation: "chat".into(),
            input_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: Some(150),
            duration_ms: 200,
            input_tokens_details: None,
            output_tokens_details: None,
        };
        let json = serde_json::to_value(&span).unwrap();
        assert_eq!(json["model"], "gpt-4");
        assert_eq!(json["input_tokens"], 100);
        assert_eq!(json["system"], "openai");
        assert_eq!(json["operation"], "chat");
    }

    #[test]
    fn test_tool_span_serialization() {
        let span = ToolSpan {
            name: "search".into(),
            success: true,
            error_type: None,
            duration_ms: 42,
        };
        let json = serde_json::to_value(&span).unwrap();
        assert_eq!(json["name"], "search");
        assert!(json["success"].as_bool().unwrap());
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
    fn test_extract_input_details() {
        let u = usage_with_details(100, 50, 150, 30, 10);
        let d = extract_input_details(Some(&u)).unwrap();
        assert_eq!(d.cached_tokens, Some(30));
        assert!(d.cache_creation_tokens.is_none());
    }

    #[test]
    fn test_extract_output_details() {
        let u = usage_with_details(100, 50, 150, 30, 10);
        let d = extract_output_details(Some(&u)).unwrap();
        assert_eq!(d.reasoning_tokens, Some(10));
        assert!(d.audio_tokens.is_none());
    }

    #[test]
    fn test_extract_details_none() {
        assert!(extract_input_details(None).is_none());
        assert!(extract_output_details(None).is_none());
        let u = usage(10, 20, 30);
        assert!(extract_input_details(Some(&u)).is_none());
        assert!(extract_output_details(Some(&u)).is_none());
    }

    #[test]
    fn test_plugin_id() {
        let sink = InMemorySink::new();
        let plugin = LLMMetryPlugin::new(sink);
        assert_eq!(plugin.id(), "llmmetry");
    }

    #[test]
    fn test_token_details_serialization() {
        let input = InputTokensDetails {
            cached_tokens: Some(50),
            cache_creation_tokens: Some(10),
            audio_tokens: None,
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["cached_tokens"], 50);
        assert_eq!(json["cache_creation_tokens"], 10);

        let output = OutputTokensDetails {
            reasoning_tokens: Some(20),
            audio_tokens: None,
            accepted_prediction_tokens: Some(5),
            rejected_prediction_tokens: None,
        };
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["reasoning_tokens"], 20);
        assert_eq!(json["accepted_prediction_tokens"], 5);
    }
}
