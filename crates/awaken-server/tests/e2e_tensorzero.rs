// TensorZero integration tests are not feasible in this crate because it lacks
// access to `awaken-ext-observability`.  The observability pipeline is fully
// tested via InMemorySink in:
//   crates/awaken/tests/observability_e2e.rs   (agent-loop level)
//   crates/awaken-ext-observability/tests/      (unit + integration)
//
// A live TensorZero e2e test would require the DEEPSEEK_API_KEY environment
// variable and a running TensorZero gateway.  That is covered by manual
// validation rather than CI.
