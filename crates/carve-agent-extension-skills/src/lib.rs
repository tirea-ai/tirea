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
mod subsystem;
mod tool_filter;
mod tools;
mod types;

pub const SKILLS_PLUGIN_ID: &str = "skills";
pub const SKILLS_BUNDLE_ID: &str = SKILLS_PLUGIN_ID;
pub const SKILLS_DISCOVERY_PLUGIN_ID: &str = "skills_discovery";
pub const SKILLS_RUNTIME_PLUGIN_ID: &str = "skills_runtime";

pub const SKILL_ACTIVATE_TOOL_ID: &str = "skill";
pub const SKILL_LOAD_RESOURCE_TOOL_ID: &str = "load_skill_resource";
pub const SKILL_SCRIPT_TOOL_ID: &str = "skill_script";

pub use types::{
    material_key, CompositeSkillRegistry, CompositeSkillRegistryError, LoadedAsset,
    LoadedReference, ScriptResult, SkillMaterializeError, SkillMeta, SkillRegistry,
    SkillRegistryError, SkillRegistryWarning, SkillResource, SkillResourceKind, SkillState,
    SKILLS_STATE_PATH,
};
pub use combined_plugin::SkillPlugin;
pub use discovery_plugin::SkillDiscoveryPlugin;
pub use embedded_registry::{EmbeddedSkill, EmbeddedSkillRegistry};
pub use in_memory_registry::InMemorySkillRegistry;
pub use registry::FsSkillRegistry;
pub use runtime_plugin::SkillRuntimePlugin;
pub use subsystem::{SkillSubsystem, SkillSubsystemError};
pub use tools::{LoadSkillResourceTool, SkillActivateTool, SkillScriptTool};
