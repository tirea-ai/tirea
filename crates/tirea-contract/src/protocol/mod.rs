//! Protocol adapter traits and request types.

pub mod adapter;
pub mod request;

pub use adapter::{ProtocolHistoryEncoder, ProtocolOutputEncoder};
pub use request::RunRequest;
