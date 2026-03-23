use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use awaken_contract::StateError;
use awaken_contract::model::Phase;
use awaken_runtime::{Plugin, PluginDescriptor, PluginRegistrar};

use crate::metrics::AgentMetrics;
use crate::sink::MetricsSink;

use super::hooks::{
    AfterInferenceHook, AfterToolExecuteHook, BeforeInferenceHook, BeforeToolExecuteHook,
    RunEndHook, RunStartHook,
};
use super::shared::{Inner, lock_unpoison};

/// Plugin that captures LLM and tool telemetry aligned with OpenTelemetry GenAI conventions.
pub struct ObservabilityPlugin {
    pub(crate) inner: Arc<Inner>,
}

impl ObservabilityPlugin {
    pub fn new(sink: impl MetricsSink + 'static) -> Self {
        Self {
            inner: Arc::new(Inner {
                sink: Arc::new(sink),
                run_start: Mutex::new(None),
                metrics: Mutex::new(AgentMetrics::default()),
                inference_start: Mutex::new(None),
                tool_start: Mutex::new(HashMap::new()),
                model: Mutex::new(String::new()),
                provider: Mutex::new(String::new()),
                operation: "chat".to_string(),
                temperature: Mutex::new(None),
                top_p: Mutex::new(None),
                max_tokens: Mutex::new(None),
                stop_sequences: Mutex::new(Vec::new()),
                inference_tracing_span: Mutex::new(None),
                tool_tracing_span: Mutex::new(HashMap::new()),
            }),
        }
    }

    #[must_use]
    pub fn with_model(self, model: impl Into<String>) -> Self {
        *lock_unpoison(&self.inner.model) = model.into();
        self
    }

    #[must_use]
    pub fn with_provider(self, provider: impl Into<String>) -> Self {
        *lock_unpoison(&self.inner.provider) = provider.into();
        self
    }

    #[must_use]
    pub fn with_temperature(self, temperature: f64) -> Self {
        *lock_unpoison(&self.inner.temperature) = Some(temperature);
        self
    }

    #[must_use]
    pub fn with_top_p(self, top_p: f64) -> Self {
        *lock_unpoison(&self.inner.top_p) = Some(top_p);
        self
    }

    #[must_use]
    pub fn with_max_tokens(self, max_tokens: u32) -> Self {
        *lock_unpoison(&self.inner.max_tokens) = Some(max_tokens);
        self
    }

    #[must_use]
    pub fn with_stop_sequences(self, seqs: Vec<String>) -> Self {
        *lock_unpoison(&self.inner.stop_sequences) = seqs;
        self
    }
}

impl Plugin for ObservabilityPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: "observability",
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        let id = "observability";
        let s = Arc::clone(&self.inner);
        registrar.register_phase_hook(id, Phase::RunStart, RunStartHook(Arc::clone(&s)))?;
        registrar.register_phase_hook(
            id,
            Phase::BeforeInference,
            BeforeInferenceHook(Arc::clone(&s)),
        )?;
        registrar.register_phase_hook(
            id,
            Phase::AfterInference,
            AfterInferenceHook(Arc::clone(&s)),
        )?;
        registrar.register_phase_hook(
            id,
            Phase::BeforeToolExecute,
            BeforeToolExecuteHook(Arc::clone(&s)),
        )?;
        registrar.register_phase_hook(
            id,
            Phase::AfterToolExecute,
            AfterToolExecuteHook(Arc::clone(&s)),
        )?;
        registrar.register_phase_hook(id, Phase::RunEnd, RunEndHook(Arc::clone(&s)))?;
        Ok(())
    }
}
