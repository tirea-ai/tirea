//! Bundled extensions for the awaken runtime.
//!
//! - [`a2a`]: Agent-to-Agent delegation via A2A protocol
//!- [`background`]: Background task management
//! - [`handoff`]: Dynamic same-thread agent switching

#[cfg(feature = "a2a")]
pub mod a2a;
#[cfg(feature = "background")]
pub mod background;
#[cfg(feature = "handoff")]
pub mod handoff;
