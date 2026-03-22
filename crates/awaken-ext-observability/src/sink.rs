use std::sync::{Arc, Mutex};

use super::metrics::{AgentMetrics, GenAISpan, ToolSpan};

/// Trait for consuming telemetry data.
pub trait MetricsSink: Send + Sync {
    fn on_inference(&self, span: &GenAISpan);
    fn on_tool(&self, span: &ToolSpan);
    fn on_run_end(&self, metrics: &AgentMetrics);
}

/// In-memory sink for testing and inspection.
#[derive(Debug, Clone, Default)]
pub struct InMemorySink {
    inner: Arc<Mutex<AgentMetrics>>,
}

impl InMemorySink {
    pub fn new() -> Self {
        Self::default()
    }

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

    fn on_run_end(&self, metrics: &AgentMetrics) {
        self.inner.lock().unwrap().session_duration_ms = metrics.session_duration_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_genai_span(model: &str, input: i32, output: i32) -> GenAISpan {
        GenAISpan {
            model: model.to_string(),
            provider: "test-provider".to_string(),
            operation: "chat".to_string(),
            response_model: None,
            response_id: None,
            finish_reasons: vec!["end_turn".to_string()],
            error_type: None,
            error_class: None,
            thinking_tokens: None,
            input_tokens: Some(input),
            output_tokens: Some(output),
            total_tokens: Some(input + output),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: Vec::new(),
            duration_ms: 100,
        }
    }

    fn sample_tool_span(name: &str, error: bool) -> ToolSpan {
        ToolSpan {
            name: name.to_string(),
            operation: "execute".to_string(),
            call_id: "call_1".to_string(),
            tool_type: "function".to_string(),
            error_type: if error {
                Some("timeout".to_string())
            } else {
                None
            },
            duration_ms: 50,
        }
    }

    #[test]
    fn in_memory_sink_records_inferences() {
        let sink = InMemorySink::new();
        sink.on_inference(&sample_genai_span("gpt-4", 100, 50));
        sink.on_inference(&sample_genai_span("gpt-4", 200, 75));

        let metrics = sink.metrics();
        assert_eq!(metrics.inferences.len(), 2);
        assert_eq!(metrics.total_input_tokens(), 300);
        assert_eq!(metrics.total_output_tokens(), 125);
    }

    #[test]
    fn in_memory_sink_records_tools() {
        let sink = InMemorySink::new();
        sink.on_tool(&sample_tool_span("read_file", false));
        sink.on_tool(&sample_tool_span("write_file", true));

        let metrics = sink.metrics();
        assert_eq!(metrics.tools.len(), 2);
        assert_eq!(metrics.tool_failures(), 1);
    }

    #[test]
    fn in_memory_sink_records_session_duration() {
        let sink = InMemorySink::new();
        let run_metrics = AgentMetrics {
            inferences: Vec::new(),
            tools: Vec::new(),
            session_duration_ms: 5000,
        };
        sink.on_run_end(&run_metrics);

        let metrics = sink.metrics();
        assert_eq!(metrics.session_duration_ms, 5000);
    }

    #[test]
    fn in_memory_sink_starts_empty() {
        let sink = InMemorySink::new();
        let metrics = sink.metrics();
        assert!(metrics.inferences.is_empty());
        assert!(metrics.tools.is_empty());
        assert_eq!(metrics.session_duration_ms, 0);
    }

    #[test]
    fn in_memory_sink_clone_shares_data() {
        let sink = InMemorySink::new();
        let clone = sink.clone();
        sink.on_inference(&sample_genai_span("model-a", 10, 5));

        let metrics = clone.metrics();
        assert_eq!(metrics.inferences.len(), 1);
    }

    #[test]
    fn in_memory_sink_stats_by_model() {
        let sink = InMemorySink::new();
        sink.on_inference(&sample_genai_span("gpt-4", 100, 50));
        sink.on_inference(&sample_genai_span("gpt-4", 200, 75));
        sink.on_inference(&sample_genai_span("claude-3", 150, 60));

        let metrics = sink.metrics();
        let stats = metrics.stats_by_model();
        assert_eq!(stats.len(), 2);

        let claude = stats.iter().find(|s| s.model == "claude-3").unwrap();
        assert_eq!(claude.inference_count, 1);
        assert_eq!(claude.input_tokens, 150);

        let gpt = stats.iter().find(|s| s.model == "gpt-4").unwrap();
        assert_eq!(gpt.inference_count, 2);
        assert_eq!(gpt.input_tokens, 300);
    }

    #[test]
    fn in_memory_sink_stats_by_tool() {
        let sink = InMemorySink::new();
        sink.on_tool(&sample_tool_span("read_file", false));
        sink.on_tool(&sample_tool_span("read_file", false));
        sink.on_tool(&sample_tool_span("write_file", true));

        let metrics = sink.metrics();
        let stats = metrics.stats_by_tool();
        assert_eq!(stats.len(), 2);

        let read = stats.iter().find(|s| s.name == "read_file").unwrap();
        assert_eq!(read.call_count, 2);
        assert_eq!(read.failure_count, 0);

        let write = stats.iter().find(|s| s.name == "write_file").unwrap();
        assert_eq!(write.call_count, 1);
        assert_eq!(write.failure_count, 1);
    }
}
