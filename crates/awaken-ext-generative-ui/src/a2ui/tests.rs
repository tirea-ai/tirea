use awaken_contract::contract::tool::{Tool, ToolCallContext};
use serde_json::json;

use awaken_runtime::plugins::Plugin;

use super::*;

const TEST_CATALOG: &str = "https://a2ui.org/specification/v0_8/standard_catalog_definition.json";

#[test]
fn valid_begin_rendering() {
    let msgs = vec![json!({
        "beginRendering": {
            "surfaceId": "s1",
            "root": "root",
        }
    })];
    assert!(validate_a2ui_messages(&msgs).is_empty());
}

#[test]
fn valid_surface_update() {
    let msgs = vec![json!({
        "surfaceUpdate": {
            "surfaceId": "s1",
            "components": [
                {"id": "root", "component": {"Card": {"child": "content"}}},
                {"id": "content", "component": {"Text": {"text": {"literalString": "Hello"}}}}
            ]
        }
    })];
    assert!(validate_a2ui_messages(&msgs).is_empty());
}

#[test]
fn valid_data_model_update() {
    let msgs = vec![json!({
        "dataModelUpdate": {
            "surfaceId": "s1",
            "path": "/request",
            "contents": [
                {"key": "requester", "valueString": "Alice"},
                {"key": "priority", "valueNumber": 2.0}
            ]
        }
    })];
    assert!(validate_a2ui_messages(&msgs).is_empty());
}

#[test]
fn valid_delete_surface() {
    let msgs = vec![json!({
        "deleteSurface": {"surfaceId": "s1"}
    })];
    assert!(validate_a2ui_messages(&msgs).is_empty());
}

#[test]
fn rejects_no_message_type() {
    let msgs = vec![json!({})];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("missing message type"));
}

#[test]
fn rejects_multiple_message_types() {
    let msgs = vec![json!({
        "beginRendering": {"surfaceId": "s1", "root": "root"},
        "deleteSurface": {"surfaceId": "s1"}
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("multiple message types"));
}

#[test]
fn rejects_missing_surface_id() {
    let msgs = vec![json!({
        "deleteSurface": {}
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("surfaceId"));
}

#[test]
fn rejects_empty_surface_id() {
    let msgs = vec![json!({
        "beginRendering": {"surfaceId": "", "root": "root"}
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("must not be empty"));
}

#[test]
fn rejects_missing_root() {
    let msgs = vec![json!({
        "beginRendering": {"surfaceId": "s1"}
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("root"));
}

#[test]
fn rejects_empty_components() {
    let msgs = vec![json!({
        "surfaceUpdate": {"surfaceId": "s1", "components": []}
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("must not be empty"));
}

#[test]
fn rejects_component_missing_id() {
    let msgs = vec![json!({
        "surfaceUpdate": {
            "surfaceId": "s1",
            "components": [{"component": {"Text": {"text": {"literalString": "hi"}}}}]
        }
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("components[0].id"));
}

#[test]
fn rejects_component_missing_component_payload() {
    let msgs = vec![json!({
        "surfaceUpdate": {
            "surfaceId": "s1",
            "components": [{"id": "root"}]
        }
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("components[0].component"));
}

#[test]
fn rejects_empty_data_model_contents() {
    let msgs = vec![json!({
        "dataModelUpdate": {
            "surfaceId": "s1",
            "contents": []
        }
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("contents"));
}

#[test]
fn rejects_data_model_entry_missing_key() {
    let msgs = vec![json!({
        "dataModelUpdate": {
            "surfaceId": "s1",
            "contents": [{"valueString": "hi"}]
        }
    })];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("contents[0].key"));
}

#[test]
fn reports_per_message_errors() {
    let msgs = vec![
        json!({"deleteSurface": {"surfaceId": "ok"}}),
        json!({}),
        json!({"deleteSurface": {"surfaceId": "ok"}}),
        json!(42),
    ];
    let errs = validate_a2ui_messages(&msgs);
    assert_eq!(errs.len(), 2);
    assert_eq!(errs[0].index, 1);
    assert_eq!(errs[1].index, 3);
}

#[test]
fn tool_descriptor_has_correct_id() {
    let tool = A2uiRenderTool::new();
    let desc = tool.descriptor();
    assert_eq!(desc.id, A2UI_TOOL_ID);
    assert_eq!(desc.name, A2UI_TOOL_NAME);
    assert!(desc.description.contains("A2UI"));
}

#[test]
fn tool_descriptor_exposes_strict_nested_schema() {
    let tool = A2uiRenderTool::new();
    let desc = tool.descriptor();
    let schema = &desc.parameters;

    assert_eq!(schema["type"], json!("object"));
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(
        schema["properties"]["surfaceUpdate"]["required"],
        json!(["surfaceId", "components"])
    );
    assert_eq!(
        schema["properties"]["surfaceUpdate"]["properties"]["components"]["items"]["required"],
        json!(["id", "component"])
    );
}

#[test]
fn tool_rejects_empty_args() {
    let tool = A2uiRenderTool::new();
    assert!(tool.validate_args(&json!({})).is_err());
}

#[test]
fn tool_accepts_flat_begin_rendering() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "beginRendering": {"surfaceId": "s1", "root": "root"}
    });
    assert!(tool.validate_args(&args).is_ok());
}

#[test]
fn tool_accepts_flat_surface_update() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "surfaceUpdate": {
            "surfaceId": "s1",
            "components": [{"id": "root", "component": {"Card": {"child": "content"}}}]
        }
    });
    assert!(tool.validate_args(&args).is_ok());
}

#[test]
fn tool_accepts_flat_data_model_update() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "dataModelUpdate": {
            "surfaceId": "s1",
            "path": "/request",
            "contents": [{"key": "requester", "valueString": "Alice"}]
        }
    });
    assert!(tool.validate_args(&args).is_ok());
}

#[test]
fn tool_accepts_flat_delete_surface() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "deleteSurface": {"surfaceId": "s1"}
    });
    assert!(tool.validate_args(&args).is_ok());
}

#[test]
fn tool_rejects_structural_error() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "beginRendering": {"surfaceId": "s1"}
    });
    let err = tool.validate_args(&args).unwrap_err();
    assert!(err.to_string().contains("root"));
}

#[test]
fn tool_rejects_legacy_multiple_choice_shape() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "surfaceUpdate": {
            "surfaceId": "approval",
            "components": [
                {
                    "id": "priority",
                    "component": {
                        "MultipleChoice": {
                            "choices": [
                                {
                                    "label": { "literalString": "High" },
                                    "value": "high"
                                }
                            ],
                            "label": { "literalString": "Priority" },
                            "value": { "path": "/request/priority" }
                        }
                    }
                }
            ]
        }
    });

    let err = tool.validate_args(&args).unwrap_err();
    assert!(err.to_string().contains("choices") || err.to_string().contains("MultipleChoice"));
}

#[test]
fn tool_rejects_button_context_object_shape() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "surfaceUpdate": {
            "surfaceId": "approval",
            "components": [
                {
                    "id": "submit_text",
                    "component": {
                        "Text": {
                            "text": { "literalString": "Submit" }
                        }
                    }
                },
                {
                    "id": "submit_button",
                    "component": {
                        "Button": {
                            "child": "submit_text",
                            "action": {
                                "name": "approval.submit",
                                "context": {
                                    "requester": { "path": "/request/requester" }
                                }
                            }
                        }
                    }
                }
            ]
        }
    });

    let err = tool.validate_args(&args).unwrap_err();
    assert!(err.to_string().contains("context") || err.to_string().contains("Button"));
}

#[test]
fn tool_accepts_official_multiple_choice_and_button_shapes() {
    let tool = A2uiRenderTool::new();
    let args = json!({
        "surfaceUpdate": {
            "surfaceId": "approval",
            "components": [
                {
                    "id": "root",
                    "component": {
                        "Card": {
                            "child": "content"
                        }
                    }
                },
                {
                    "id": "content",
                    "component": {
                        "Column": {
                            "children": {
                                "explicitList": ["priority_label", "priority", "submit_text", "submit_button"]
                            },
                            "distribution": "start",
                            "alignment": "stretch"
                        }
                    }
                },
                {
                    "id": "priority_label",
                    "component": {
                        "Text": {
                            "usageHint": "caption",
                            "text": { "literalString": "Priority" }
                        }
                    }
                },
                {
                    "id": "priority",
                    "component": {
                        "MultipleChoice": {
                            "selections": { "path": "/request/priority" },
                            "options": [
                                {
                                    "label": { "literalString": "Standard" },
                                    "value": "standard"
                                }
                            ],
                            "maxAllowedSelections": 1
                        }
                    }
                },
                {
                    "id": "submit_text",
                    "component": {
                        "Text": {
                            "text": { "literalString": "Submit" }
                        }
                    }
                },
                {
                    "id": "submit_button",
                    "component": {
                        "Button": {
                            "child": "submit_text",
                            "action": {
                                "name": "approval.submit",
                                "context": [
                                    {
                                        "key": "priority",
                                        "value": { "path": "/request/priority" }
                                    }
                                ]
                            }
                        }
                    }
                }
            ]
        }
    });

    assert!(tool.validate_args(&args).is_ok());
}

#[tokio::test]
async fn tool_execute_flat_args() {
    let tool = A2uiRenderTool::new();
    let ctx = ToolCallContext::test_default();
    let args = json!({
        "beginRendering": {"surfaceId": "s1", "root": "root"}
    });
    let result = tool.execute(args, &ctx).await.unwrap();
    assert_eq!(result.data["rendered"], true);
    assert_eq!(result.data["count"], 1);
}

#[test]
fn tool_accepts_legacy_messages_array() {
    let tool = A2uiRenderTool::new();
    let args = json!({"messages": [
        {"surfaceUpdate": {"surfaceId": "s1", "components": [{"id": "root", "component": {"Card": {"child": "content"}}}]}},
        {"beginRendering": {"surfaceId": "s1", "root": "root"}}
    ]});
    assert!(tool.validate_args(&args).is_ok());
}

#[test]
fn tool_rejects_empty_messages_array() {
    let tool = A2uiRenderTool::new();
    let err = tool.validate_args(&json!({"messages": []})).unwrap_err();
    assert!(err.to_string().contains("at least one"));
}

#[tokio::test]
async fn tool_execute_legacy_messages_array() {
    let tool = A2uiRenderTool::new();
    let ctx = ToolCallContext::test_default();
    let args = json!({"messages": [
        {"surfaceUpdate": {"surfaceId": "s1", "components": [{"id": "root", "component": {"Card": {"child": "content"}}}]}},
        {"beginRendering": {"surfaceId": "s1", "root": "root"}}
    ]});
    let result = tool.execute(args, &ctx).await.unwrap();
    assert_eq!(result.tool_name, A2UI_TOOL_NAME);
    assert_eq!(result.data["rendered"], true);
    assert_eq!(result.data["count"], 2);
}

#[test]
fn plugin_id() {
    let plugin = A2uiPlugin::with_catalog_id(TEST_CATALOG);
    assert_eq!(plugin.descriptor().name, A2UI_PLUGIN_ID);
}

#[test]
fn plugin_instructions_contain_catalog() {
    let plugin = A2uiPlugin::with_catalog_id(TEST_CATALOG);
    assert!(plugin.instructions().contains(TEST_CATALOG));
    assert!(!plugin.instructions().contains("{{CATALOG_ID}}"));
}

#[test]
fn plugin_with_examples() {
    let plugin = A2uiPlugin::with_catalog_and_examples(TEST_CATALOG, "example content");
    assert!(plugin.instructions().contains("---BEGIN A2UI EXAMPLES---"));
    assert!(plugin.instructions().contains("example content"));
    assert!(plugin.instructions().contains("---END A2UI EXAMPLES---"));
}

#[test]
fn plugin_custom_instructions() {
    let plugin = A2uiPlugin::with_custom_instructions("Use A2UI.".into());
    assert_eq!(plugin.instructions(), "Use A2UI.");
}

#[test]
fn plugin_instructions_mention_all_message_types() {
    let plugin = A2uiPlugin::with_catalog_id(TEST_CATALOG);
    let text = plugin.instructions();
    assert!(text.contains("render_a2ui"));
    assert!(text.contains("surfaceUpdate"));
    assert!(text.contains("dataModelUpdate"));
    assert!(text.contains("beginRendering"));
    assert!(text.contains("deleteSurface"));
}
