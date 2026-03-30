use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::{Value, json};

use awaken_contract::contract::tool::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolOutput, ToolResult,
};
use awaken_ext_generative_ui::{json_render, run_streaming_subagent};
use awaken_runtime::AgentResolver;

#[derive(Clone, Default)]
pub struct SharedAgentResolver {
    inner: Arc<RwLock<Option<Arc<dyn AgentResolver>>>>,
}

impl SharedAgentResolver {
    pub fn set(&self, resolver: Arc<dyn AgentResolver>) {
        *self.inner.write().expect("resolver lock poisoned") = Some(resolver);
    }

    fn get(&self) -> Result<Arc<dyn AgentResolver>, ToolError> {
        self.inner
            .read()
            .expect("resolver lock poisoned")
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                ToolError::ExecutionFailed("generative UI resolver is not initialized".to_string())
            })
    }
}

#[derive(Clone, Copy)]
enum SnapshotFormat {
    JsonRender,
    OpenUi,
}

pub struct StreamingGenerativeUiTool {
    descriptor: ToolDescriptor,
    subagent_id: &'static str,
    snapshot_format: SnapshotFormat,
    resolver: SharedAgentResolver,
}

impl StreamingGenerativeUiTool {
    fn new(
        id: &'static str,
        name: &'static str,
        description: &'static str,
        subagent_id: &'static str,
        snapshot_format: SnapshotFormat,
        resolver: SharedAgentResolver,
    ) -> Self {
        Self {
            descriptor: ToolDescriptor::new(id, name, description).with_parameters(json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Business-oriented description of the UI to generate"
                    }
                },
                "required": ["prompt"],
                "additionalProperties": false
            })),
            subagent_id,
            snapshot_format,
            resolver,
        }
    }

    pub fn json_render(resolver: SharedAgentResolver) -> Self {
        Self::new(
            "render_json_ui",
            "Render JSON UI",
            "Generate a JSON Render interface through a streaming UI sub-agent.",
            "json-render-ui",
            SnapshotFormat::JsonRender,
            resolver,
        )
    }

    pub fn openui(resolver: SharedAgentResolver) -> Self {
        Self::new(
            "render_openui_ui",
            "Render OpenUI UI",
            "Generate an OpenUI Lang interface through a streaming UI sub-agent.",
            "openui-ui",
            SnapshotFormat::OpenUi,
            resolver,
        )
    }

    fn finalize_output(&self, content: &str) -> Result<Value, ToolError> {
        match self.snapshot_format {
            SnapshotFormat::JsonRender => json_render::compile_output(content).map_err(|error| {
                ToolError::ExecutionFailed(format!(
                    "json-render sub-agent returned invalid SpecStream output: {error}"
                ))
            }),
            SnapshotFormat::OpenUi => Ok(Value::String(content.to_string())),
        }
    }
}

#[async_trait]
impl Tool for StreamingGenerativeUiTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    fn validate_args(&self, args: &Value) -> Result<(), ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if prompt.is_none() {
            return Err(ToolError::InvalidArguments(
                "missing required non-empty field \"prompt\"".into(),
            ));
        }
        Ok(())
    }

    async fn execute(&self, args: Value, ctx: &ToolCallContext) -> Result<ToolOutput, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required field \"prompt\"".into())
            })?;
        if prompt.is_empty() {
            return Err(ToolError::InvalidArguments(
                "field \"prompt\" cannot be empty".into(),
            ));
        }

        let resolver = self.resolver.get()?;
        let result =
            run_streaming_subagent(resolver.as_ref(), self.subagent_id, prompt, ctx).await?;

        let snapshot = self.finalize_output(&result.content)?;

        Ok(ToolResult::success(
            &self.descriptor.id,
            json!({
                "content": snapshot,
                "steps": result.steps,
            }),
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_args_rejects_missing_prompt() {
        let tool = StreamingGenerativeUiTool::json_render(SharedAgentResolver::default());
        let error = tool.validate_args(&json!({})).unwrap_err();
        assert!(matches!(error, ToolError::InvalidArguments(_)));
    }

    #[test]
    fn validate_args_rejects_empty_prompt() {
        let tool = StreamingGenerativeUiTool::openui(SharedAgentResolver::default());
        let error = tool.validate_args(&json!({ "prompt": "   " })).unwrap_err();
        assert!(matches!(error, ToolError::InvalidArguments(_)));
    }

    #[test]
    fn finalize_json_render_compiles_spec_stream() {
        let tool = StreamingGenerativeUiTool::json_render(SharedAgentResolver::default());
        let output = tool
            .finalize_output(
                r#"{"op":"add","path":"/root","value":"workspace"}
{"op":"add","path":"/elements/workspace","value":{"type":"Card","props":{"title":"Quarterly review"},"children":[]}}"#,
            )
            .expect("spec stream should compile");

        assert_eq!(output["root"], "workspace");
        assert_eq!(
            output["elements"]["workspace"]["props"]["title"],
            "Quarterly review"
        );
    }
}
