use crate::plugins::PermissionContextExt;
use crate::skills::materialize::{
    load_reference_material, run_script_material, SkillMaterializeError,
};
use crate::skills::registry::SkillRegistry;
use crate::skills::skill_md::parse_skill_md;
use crate::skills::state::{material_key, SkillState, SKILLS_STATE_PATH};
use crate::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_state::Context;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SkillActivateTool {
    registry: Arc<SkillRegistry>,
}

impl SkillActivateTool {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
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

        let skill_md_path = self.registry.skill_md_path(&meta);
        let raw = tokio::task::spawn_blocking(move || std::fs::read_to_string(skill_md_path))
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let doc = parse_skill_md(&raw);
        let instructions = doc.body;

        let state = ctx.state::<SkillState>(SKILLS_STATE_PATH);
        let active = state.active().ok().unwrap_or_default();
        if !active.iter().any(|s| s == &meta.id) {
            state.active_push(meta.id.clone());
        }
        state.instructions_insert(meta.id.clone(), instructions);

        // Apply allowed-tools to the existing permission model (no scope).
        for t in doc.frontmatter.allowed_tools {
            ctx.allow_tool(t);
        }

        Ok(ToolResult::success(
            "skill",
            json!({
                "activated": true,
                "skill_id": meta.id,
                "name": meta.name,
                "allowed_tools_applied": meta.allowed_tools,
            }),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct LoadSkillReferenceTool {
    registry: Arc<SkillRegistry>,
}

impl LoadSkillReferenceTool {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
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

        let mat = tokio::task::spawn_blocking({
            let root = meta.root_dir.clone();
            let skill_id = meta.id.clone();
            move || load_reference_material(&skill_id, &root, &path)
        })
        .await
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
        .map_err(map_materialize_error)?;

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
    registry: Arc<SkillRegistry>,
}

impl SkillScriptTool {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
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

        let res = run_script_material(&meta.id, &meta.root_dir, &script, &argv)
            .await
            .map_err(map_materialize_error)?;

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

fn map_materialize_error(e: SkillMaterializeError) -> ToolError {
    match e {
        SkillMaterializeError::InvalidPath(msg) => ToolError::InvalidArguments(msg),
        SkillMaterializeError::UnsupportedPath(msg) => ToolError::InvalidArguments(msg),
        SkillMaterializeError::PathEscapesRoot => {
            ToolError::InvalidArguments("path escapes root".to_string())
        }
        SkillMaterializeError::UnsupportedRuntime(p) => {
            ToolError::InvalidArguments(format!("unsupported runtime: {p}"))
        }
        SkillMaterializeError::Timeout(secs) => {
            ToolError::ExecutionFailed(format!("timeout after {secs}s"))
        }
        SkillMaterializeError::Io(msg) => ToolError::ExecutionFailed(msg),
    }
}
