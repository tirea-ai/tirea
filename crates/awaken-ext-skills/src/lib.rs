//! Skill subsystem for awaken.
//!
//! This module provides:
//! - `Skill`: per-skill trait with IO capabilities (read instructions, load resources, run scripts)
//! - `FsSkill`: filesystem-backed skill with directory discovery
//! - `EmbeddedSkill`: compile-time embedded skill from static content
//! - Tools: activate skill, load reference, run script
//! - `SkillDiscoveryPlugin`: inject skills catalog before inference

mod embedded;
pub mod error;
mod materialize;
pub mod plugin;
pub mod registry;
pub mod skill;
pub mod skill_md;
pub mod state;
pub mod tools;

/// Re-export the `AddContextMessage` scheduled action type from awaken-runtime.
pub(crate) use awaken_runtime::agent::state::AddContextMessage;

pub const SKILLS_PLUGIN_ID: &str = "skills";
pub const SKILLS_BUNDLE_ID: &str = SKILLS_PLUGIN_ID;
pub const SKILLS_DISCOVERY_PLUGIN_ID: &str = "skills_discovery";
pub const SKILLS_ACTIVE_INSTRUCTIONS_PLUGIN_ID: &str = "skills_active_instructions";

pub const SKILL_ACTIVATE_TOOL_ID: &str = "skill";
pub const SKILL_LOAD_RESOURCE_TOOL_ID: &str = "load_skill_resource";
pub const SKILL_SCRIPT_TOOL_ID: &str = "skill_script";

pub use embedded::{EmbeddedSkill, EmbeddedSkillData};
pub use error::{
    SkillError, SkillMaterializeError, SkillRegistryError, SkillRegistryManagerError, SkillWarning,
};
pub use plugin::{
    ActiveSkillInstructionsPlugin, SkillDiscoveryPlugin, SkillSubsystem, SkillSubsystemError,
};
pub use registry::{
    CompositeSkillRegistry, DiscoveryResult, FsSkill, FsSkillRegistryManager,
    InMemorySkillRegistry, SkillRegistry,
};
pub use skill::{
    LoadedAsset, LoadedReference, ScriptResult, Skill, SkillActivation, SkillMeta, SkillResource,
    SkillResourceKind, collect_skills, material_key,
};
pub use state::{SkillState, SkillStateUpdate, SkillStateValue};
pub use tools::{LoadSkillResourceTool, SkillActivateTool, SkillScriptTool};
