//! Protocol transcoder traits and request types.

pub mod request;
pub mod transcoder;

pub use request::{RunRequest, RuntimeInput};
pub use transcoder::{Identity, Transcoder};
