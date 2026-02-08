//! Skill subsystem (agentskills-style).
//!
//! This module provides:
//! - `SkillRegistry`: discover skills (metadata-only)
//! - Tools: activate skill, load reference, run script
//! - `SkillRuntimePlugin`: inject activated skill context before inference

mod materialize;
mod registry;
mod runtime_plugin;
mod skill_md;
mod state;
mod tools;

pub use registry::{SkillMeta, SkillRegistry};
pub use runtime_plugin::SkillRuntimePlugin;
pub use tools::{LoadSkillReferenceTool, SkillActivateTool, SkillScriptTool};
