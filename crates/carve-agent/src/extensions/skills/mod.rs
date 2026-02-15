//! Skill subsystem (agentskills-style).
//!
//! This module provides:
//! - `SkillRegistry`: skills catalog + materialization (pluggable backend)
//! - Tools: activate skill, load reference, run script
//! - `SkillRuntimePlugin`: inject activated skill context before inference

mod combined_plugin;
mod discovery_plugin;
mod embedded_registry;
mod in_memory_registry;
mod materialize;
mod registry;
mod resource_lookup;
mod runtime_plugin;
mod skill_md;
pub mod state;
mod subsystem;
mod tools;

pub use carve_agent_contract::skills::{
    CompositeSkillRegistry, CompositeSkillRegistryError, LoadedAsset, LoadedReference,
    ScriptResult, SkillMaterializeError, SkillMeta, SkillRegistry, SkillRegistryError,
    SkillRegistryWarning, SkillResource, SkillResourceKind,
};
pub use combined_plugin::SkillPlugin;
pub use discovery_plugin::SkillDiscoveryPlugin;
pub use embedded_registry::{EmbeddedSkill, EmbeddedSkillRegistry};
pub use in_memory_registry::InMemorySkillRegistry;
pub use registry::FsSkillRegistry;
pub use runtime_plugin::SkillRuntimePlugin;
pub use subsystem::{SkillSubsystem, SkillSubsystemError};
pub use tools::{LoadSkillResourceTool, SkillActivateTool, SkillScriptTool};
