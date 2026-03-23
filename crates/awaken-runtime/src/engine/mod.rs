//! Engine layer: genai-backed LLM executor and type conversion.
//!
//! Bridges awaken's provider-neutral types to the `genai` crate.
//! - `convert`: Message, Tool, Usage, StopReason conversions
//! - `streaming`: StreamCollector for accumulating ChatStreamEvents
//! - `executor`: `GenaiExecutor` implementing `LlmExecutor`

pub mod convert;
pub mod executor;
pub mod mock;
pub mod retry;
pub mod streaming;

pub use executor::GenaiExecutor;
pub use mock::MockLlmExecutor;
pub use retry::{LlmRetryPolicy, RetryConfigKey, RetryingExecutor};
