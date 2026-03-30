// Phoenix/OTLP observability integration tests.
//
// The OtelMetricsSink attribute mapping is tested at the unit level in:
//   crates/awaken-ext-observability/src/otel.rs (unit tests)
//
// The full metrics pipeline (agent loop -> ObservabilityPlugin -> InMemorySink)
// is tested without external dependencies in:
//   crates/awaken/tests/observability_e2e.rs
//
// This file is reserved for live Phoenix OTLP endpoint tests that verify
// spans appear in a real Phoenix instance. These require:
//   - PHOENIX_BASE_URL or OTEL_EXPORTER_OTLP_ENDPOINT env var
//   - A running Phoenix collector accepting OTLP/gRPC or OTLP/HTTP
