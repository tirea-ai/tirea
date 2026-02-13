//! Skill subsystem (agentskills-style).
//!
//! This module provides:
//! - `SkillRegistry`: skills catalog + materialization (pluggable backend)
//! - Tools: activate skill, load reference, run script
//! - `SkillRuntimePlugin`: inject activated skill context before inference

mod combined_plugin;
mod composite_registry;
mod discovery_plugin;
mod embedded_registry;
mod in_memory_registry;
mod materialize;
mod registry;
mod runtime_plugin;
mod skill_md;
pub mod state;
mod subsystem;
mod tools;

pub use combined_plugin::SkillPlugin;
pub use composite_registry::{CompositeSkillRegistry, CompositeSkillRegistryError};
pub use discovery_plugin::SkillDiscoveryPlugin;
pub use embedded_registry::{EmbeddedSkill, EmbeddedSkillRegistry};
pub use in_memory_registry::InMemorySkillRegistry;
pub use registry::{
    FsSkillRegistry, SkillMeta, SkillRegistry, SkillRegistryError, SkillRegistryWarning,
    SkillResource, SkillResourceKind,
};
pub use runtime_plugin::SkillRuntimePlugin;
pub use subsystem::{SkillSubsystem, SkillSubsystemError};
pub use tools::{
    LoadSkillResourceTool, SkillActivateTool, SkillScriptTool, APPEND_USER_MESSAGES_METADATA_KEY,
};
