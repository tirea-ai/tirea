use async_trait::async_trait;
use carve_agentos::contracts::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agentos::contracts::AgentState;
use serde_json::{json, Value};

use super::state::{LogEntry, ResearchState, Resource};

/// Search the web for information on a research topic.
///
/// Corresponds to CopilotKit's `Search` tool (mock Tavily).
pub struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "search",
            "Search",
            "Search the web for research information",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                }
            },
            "required": ["query"]
        }))
    }

    async fn execute(&self, args: Value, ctx: &AgentState) -> Result<ToolResult, ToolError> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'query'".into()))?;

        let state = ctx.state::<ResearchState>("");

        // Log search start
        let mut logs = state.logs().unwrap_or_default();
        logs.push(LogEntry {
            message: format!("Searching for: {query}"),
            level: "info".into(),
            step: "search".into(),
        });
        state.set_logs(logs.clone());

        // Mock search results
        let results = mock_search_results(query);

        logs.push(LogEntry {
            message: format!("Found {} results for: {query}", results.len()),
            level: "info".into(),
            step: "search".into(),
        });
        state.set_logs(logs);

        Ok(ToolResult::success(
            "search",
            json!({
                "query": query,
                "results": results,
            }),
        ))
    }
}

/// Write or update the research report.
pub struct WriteReportTool;

#[async_trait]
impl Tool for WriteReportTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "write_report",
            "Write Report",
            "Write or update the research report",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "report": {
                    "type": "string",
                    "description": "The full report content in markdown"
                }
            },
            "required": ["report"]
        }))
    }

    async fn execute(&self, args: Value, ctx: &AgentState) -> Result<ToolResult, ToolError> {
        let report = args["report"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'report'".into()))?;

        let state = ctx.state::<ResearchState>("");
        state.set_report(report.to_string());

        let mut logs = state.logs().unwrap_or_default();
        logs.push(LogEntry {
            message: format!("Report updated ({} chars)", report.len()),
            level: "info".into(),
            step: "write_report".into(),
        });
        state.set_logs(logs);

        Ok(ToolResult::success(
            "write_report",
            json!({ "length": report.len() }),
        ))
    }
}

/// Set the research question.
pub struct SetQuestionTool;

#[async_trait]
impl Tool for SetQuestionTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "set_research_question",
            "Set Research Question",
            "Set or update the research question",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The research question"
                }
            },
            "required": ["question"]
        }))
    }

    async fn execute(&self, args: Value, ctx: &AgentState) -> Result<ToolResult, ToolError> {
        let question = args["question"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'question'".into()))?;

        let state = ctx.state::<ResearchState>("");
        state.set_research_question(question.to_string());

        let mut logs = state.logs().unwrap_or_default();
        logs.push(LogEntry {
            message: format!("Research question set: {question}"),
            level: "info".into(),
            step: "set_question".into(),
        });
        state.set_logs(logs);

        Ok(ToolResult::success(
            "set_research_question",
            json!({ "question": question }),
        ))
    }
}

/// Delete resources from the resource list.
///
/// Requires HITL confirmation (Pattern 2).
pub struct DeleteResourcesTool;

#[async_trait]
impl Tool for DeleteResourcesTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "delete_resources",
            "Delete Resources",
            "Delete resources from the research collection",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "resource_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "IDs of resources to delete"
                }
            },
            "required": ["resource_ids"]
        }))
        .with_confirmation(true)
    }

    async fn execute(&self, args: Value, ctx: &AgentState) -> Result<ToolResult, ToolError> {
        let ids: Vec<&str> = args["resource_ids"]
            .as_array()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'resource_ids' array".into()))?
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        let state = ctx.state::<ResearchState>("");
        let mut resources = state.resources().unwrap_or_default();
        let before = resources.len();
        resources.retain(|r| !ids.contains(&r.id.as_str()));
        let deleted = before - resources.len();
        state.set_resources(resources);

        let mut logs = state.logs().unwrap_or_default();
        logs.push(LogEntry {
            message: format!("Deleted {deleted} resources"),
            level: "info".into(),
            step: "delete_resources".into(),
        });
        state.set_logs(logs);

        Ok(ToolResult::success(
            "delete_resources",
            json!({ "deleted": deleted }),
        ))
    }
}

/// Extract resources from search results and add them to the collection.
pub struct ExtractResourcesTool;

#[async_trait]
impl Tool for ExtractResourcesTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "extract_resources",
            "Extract Resources",
            "Extract and add resources from search results",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "resources": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id":          { "type": "string" },
                            "url":         { "type": "string" },
                            "title":       { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["id", "url", "title", "description"]
                    },
                    "description": "Resources to add"
                }
            },
            "required": ["resources"]
        }))
    }

    async fn execute(&self, args: Value, ctx: &AgentState) -> Result<ToolResult, ToolError> {
        let raw = args["resources"]
            .as_array()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'resources' array".into()))?;

        let state = ctx.state::<ResearchState>("");
        let mut resources = state.resources().unwrap_or_default();

        for item in raw {
            resources.push(Resource {
                id: item["id"].as_str().unwrap_or_default().to_string(),
                url: item["url"].as_str().unwrap_or_default().to_string(),
                title: item["title"].as_str().unwrap_or_default().to_string(),
                description: item["description"].as_str().unwrap_or_default().to_string(),
            });
        }

        state.set_resources(resources.clone());

        let mut logs = state.logs().unwrap_or_default();
        logs.push(LogEntry {
            message: format!("Extracted {} resources", raw.len()),
            level: "info".into(),
            step: "extract_resources".into(),
        });
        state.set_logs(logs);

        Ok(ToolResult::success(
            "extract_resources",
            json!({ "added": raw.len(), "total": resources.len() }),
        ))
    }
}

// ---------------------------------------------------------------------------
// Mock data
// ---------------------------------------------------------------------------

fn mock_search_results(query: &str) -> Vec<Value> {
    let prefix = uuid::Uuid::new_v4().to_string()[..8].to_string();
    vec![
        json!({
            "id": format!("{prefix}-1"),
            "url": format!("https://example.com/article-1?q={}", urlencoded(query)),
            "title": format!("Understanding {query}: A Comprehensive Guide"),
            "snippet": format!("This article explores the key aspects of {query}...")
        }),
        json!({
            "id": format!("{prefix}-2"),
            "url": format!("https://example.com/article-2?q={}", urlencoded(query)),
            "title": format!("{query} - Latest Research and Findings"),
            "snippet": format!("Recent studies on {query} have shown...")
        }),
        json!({
            "id": format!("{prefix}-3"),
            "url": format!("https://example.com/article-3?q={}", urlencoded(query)),
            "title": format!("Expert Analysis: The Impact of {query}"),
            "snippet": format!("Experts weigh in on {query} and its implications...")
        }),
    ]
}

fn urlencoded(s: &str) -> String {
    s.replace(' ', "+")
}
