//! A2UI (Agent-to-UI) extension for Tirea.
//!
//! Provides a tool-based interface for agents to emit [A2UI](https://a2ui.org)
//! declarative UI messages. The LLM calls [`A2uiRenderTool`] with A2UI JSON
//! payloads; the tool validates the structure and returns the validated payload
//! as its result, which flows through the AG-UI event stream to the frontend.
//!
//! # Protocol overview
//!
//! A2UI defines four server-to-client message types:
//!
//! - `createSurface` — initialize a rendering surface
//! - `updateComponents` — define/update the component tree
//! - `updateDataModel` — populate or change data values
//! - `deleteSurface` — remove a surface
//!
//! Each message carries `"version": "v0.9"` and exactly one of the above keys.

mod plugin;
mod tool;
mod validate;

pub use plugin::A2uiPlugin;
pub use tool::A2uiRenderTool;
pub use validate::{validate_a2ui_messages, A2uiValidationError};
