use crate::plugins::PermissionContextExt;
use crate::skills::registry::{SkillRegistry, SkillRegistryError};
use crate::skills::skill_md::parse_skill_md;
use crate::skills::state::{material_key, SkillState, SKILLS_STATE_PATH};
use crate::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_state::Context;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SkillActivateTool {
    registry: Arc<dyn SkillRegistry>,
}

impl SkillActivateTool {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl Tool for SkillActivateTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "skill",
            "Skill",
            "Activate a skill and persist its instructions",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string", "description": "Skill id or name" },
                "args": { "type": "string", "description": "Optional arguments for the skill" }
            },
            "required": ["skill"]
        }))
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let key = args
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'skill'".to_string()))?
            .trim()
            .to_string();

        let meta = self
            .registry
            .resolve(&key)
            .ok_or_else(|| ToolError::NotFound(format!("Unknown skill: {key}")))?;

        let raw = self
            .registry
            .read_skill_md(&meta.id)
            .await
            .map_err(map_registry_error)?;

        let doc = parse_skill_md(&raw)
            .map_err(|e| ToolError::ExecutionFailed(format!("invalid SKILL.md: {e}")))?;
        let instructions = doc.body;

        let state = ctx.state::<SkillState>(SKILLS_STATE_PATH);
        let active = state.active().ok().unwrap_or_default();
        if !active.iter().any(|s| s == &meta.id) {
            state.active_push(meta.id.clone());
        }
        state.instructions_insert(meta.id.clone(), instructions);

        // Apply allowed-tools to the existing permission model (no scope).
        // Spec format: a space-delimited string. Tokens may include parameters, e.g. "Bash(git:*)".
        // For the current permission model we best-effort map each token to a tool id by stripping
        // optional "(...)" suffix.
        let mut applied_tool_ids: Vec<String> = Vec::new();
        for token in meta.allowed_tools.iter() {
            let tool_id = token.split('(').next().unwrap_or(token).to_string();
            if !tool_id.is_empty() {
                ctx.allow_tool(tool_id.clone());
                applied_tool_ids.push(tool_id);
            }
        }

        Ok(ToolResult::success(
            "skill",
            json!({
                "activated": true,
                "skill_id": meta.id,
                "name": meta.name,
                "allowed_tools_applied": applied_tool_ids,
            }),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct LoadSkillReferenceTool {
    registry: Arc<dyn SkillRegistry>,
}

impl LoadSkillReferenceTool {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl Tool for LoadSkillReferenceTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "load_skill_reference",
            "Load Skill Reference",
            "Load a skill reference file (references/**) into persisted state",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string", "description": "Skill id or name" },
                "path": { "type": "string", "description": "Relative path under references/** (e.g. references/OOXML.md)" }
            },
            "required": ["skill", "path"]
        }))
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let key = args
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'skill'".to_string()))?
            .trim()
            .to_string();
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'path'".to_string()))?
            .trim()
            .to_string();

        let meta = self
            .registry
            .resolve(&key)
            .ok_or_else(|| ToolError::NotFound(format!("Unknown skill: {key}")))?;

        let mat = self
            .registry
            .load_reference(&meta.id, &path)
            .await
            .map_err(map_registry_error)?;

        let key = material_key(&meta.id, &mat.path);
        let state = ctx.state::<SkillState>(SKILLS_STATE_PATH);
        state.references_insert(key, mat.clone());

        Ok(ToolResult::success(
            "load_skill_reference",
            json!({
                "loaded": true,
                "skill_id": meta.id,
                "path": mat.path,
                "bytes": mat.bytes,
                "truncated": mat.truncated,
            }),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct SkillScriptTool {
    registry: Arc<dyn SkillRegistry>,
}

impl SkillScriptTool {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl Tool for SkillScriptTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "skill_script",
            "Skill Script",
            "Run a skill script (scripts/**) and persist its result",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string", "description": "Skill id or name" },
                "script": { "type": "string", "description": "Relative path under scripts/** (e.g. scripts/run.sh)" },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Optional script arguments" }
            },
            "required": ["skill", "script"]
        }))
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let key = args
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'skill'".to_string()))?
            .trim()
            .to_string();
        let script = args
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing 'script'".to_string()))?
            .trim()
            .to_string();
        let argv: Vec<String> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let meta = self
            .registry
            .resolve(&key)
            .ok_or_else(|| ToolError::NotFound(format!("Unknown skill: {key}")))?;

        let res = self
            .registry
            .run_script(&meta.id, &script, &argv)
            .await
            .map_err(map_registry_error)?;

        let key = material_key(&meta.id, &res.script);
        let state = ctx.state::<SkillState>(SKILLS_STATE_PATH);
        state.scripts_insert(key, res.clone());

        Ok(ToolResult::success(
            "skill_script",
            json!({
                "ok": res.exit_code == 0,
                "skill_id": meta.id,
                "script": res.script,
                "exit_code": res.exit_code,
                "stdout_truncated": res.truncated_stdout,
                "stderr_truncated": res.truncated_stderr,
            }),
        ))
    }
}

fn map_registry_error(e: SkillRegistryError) -> ToolError {
    match e {
        SkillRegistryError::UnknownSkill(id) => ToolError::NotFound(format!("Unknown skill: {id}")),
        SkillRegistryError::InvalidSkillMd(msg) => ToolError::ExecutionFailed(msg),
        SkillRegistryError::Materialize(msg) => ToolError::ExecutionFailed(msg),
        SkillRegistryError::Io(msg) => ToolError::ExecutionFailed(msg),
        SkillRegistryError::DuplicateSkillId(id) => {
            ToolError::ExecutionFailed(format!("duplicate skill id: {id}"))
        }
        SkillRegistryError::Unsupported(msg) => ToolError::ExecutionFailed(msg),
    }
}
