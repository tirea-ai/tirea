//! Protocol transcoder traits and request types.

pub mod request;
pub mod transcoder;

pub use request::RunRequest;
pub use transcoder::{DecisionTranscoder, Identity, Transcoder};
