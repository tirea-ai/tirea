//! Multi-turn conversation using AgentRuntime with persistent thread storage.
//!
//! Demonstrates: each `runtime.run()` call loads history from ThreadRunStore,
//! appends new messages, runs the loop, and checkpoints the result.
//! No manual message accumulation needed.
//!
//! Run: cargo run --example multi_turn
//!
//! Requires: OPENAI_API_KEY, OPENAI_BASE_URL, OPENAI_MODEL env vars

use async_trait::async_trait;
use awaken::agent::config::AgentConfig;
use awaken::agent::loop_runner::build_agent_env;
use awaken::contract::content::ContentBlock;
use awaken::contract::event::AgentEvent;
use awaken::contract::event_sink::EventSink;
use awaken::contract::executor::{InferenceExecutionError, InferenceRequest, LlmExecutor};
use awaken::contract::inference::{StopReason, StreamResult, TokenUsage};
use awaken::contract::message::Message;
use awaken::contract::storage_mem::InMemoryThreadRunStore;
use awaken::*;
// AgentResolver, AgentRuntime, ResolvedAgent, RunRequest re-exported at crate root
use serde_json::Value;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// OpenAI-compatible executor
// ---------------------------------------------------------------------------

struct OpenAIExecutor {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAIExecutor {
    fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            api_key: std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY required"),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        }
    }
}

#[async_trait]
impl LlmExecutor for OpenAIExecutor {
    async fn execute(
        &self,
        request: InferenceRequest,
    ) -> Result<StreamResult, InferenceExecutionError> {
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    awaken::contract::message::Role::System => "system",
                    awaken::contract::message::Role::User => "user",
                    awaken::contract::message::Role::Assistant => "assistant",
                    awaken::contract::message::Role::Tool => "tool",
                };
                serde_json::json!({ "role": role, "content": m.text() })
            })
            .collect();

        let body = serde_json::json!({
            "model": if request.model.is_empty() { &self.model } else { &request.model },
            "messages": messages,
            "max_tokens": 200,
        });

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| InferenceExecutionError::Provider(e.to_string()))?;

        let text = resp
            .text()
            .await
            .map_err(|e| InferenceExecutionError::Provider(e.to_string()))?;

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| InferenceExecutionError::Provider(e.to_string()))?;

        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = json.get("usage").map(|u| TokenUsage {
            prompt_tokens: u["prompt_tokens"].as_i64().map(|v| v as i32),
            completion_tokens: u["completion_tokens"].as_i64().map(|v| v as i32),
            total_tokens: u["total_tokens"].as_i64().map(|v| v as i32),
            ..Default::default()
        });

        Ok(StreamResult {
            content: if content.is_empty() {
                vec![]
            } else {
                vec![ContentBlock::text(content)]
            },
            tool_calls: vec![],
            usage,
            stop_reason: Some(StopReason::EndTurn),
        })
    }

    fn name(&self) -> &str {
        "openai"
    }
}

// ---------------------------------------------------------------------------
// Console sink — prints deltas inline
// ---------------------------------------------------------------------------

struct ConsoleSink;

#[async_trait]
impl EventSink for ConsoleSink {
    async fn emit(&self, event: AgentEvent) {
        match &event {
            AgentEvent::TextDelta { delta } => print!("{delta}"),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Simple resolver
// ---------------------------------------------------------------------------

struct SimpleResolver {
    agent: AgentConfig,
}

impl AgentResolver for SimpleResolver {
    fn resolve(&self, _agent_id: &str) -> Result<ResolvedAgent, awaken::StateError> {
        let env = build_agent_env(&[], &self.agent)?;
        Ok(ResolvedAgent {
            config: self.agent.clone(),
            env,
        })
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let executor = OpenAIExecutor::new();
    let model_name = executor.model.clone();
    let llm = Arc::new(executor);

    let agent = AgentConfig::new(
        "chat",
        &model_name,
        "You are a helpful assistant. Be concise. Remember what the user tells you.",
        llm,
    );
    let resolver = Arc::new(SimpleResolver {
        agent: agent.clone(),
    });

    // Create runtime with persistent thread storage
    let store = Arc::new(InMemoryThreadRunStore::new());
    let runtime = AgentRuntime::new(resolver).with_thread_run_store(store);

    let thread_id = "conversation-1";

    let turns = vec![
        "My name is Alice. Remember it.",
        "What is the capital of France?",
        "What is my name?", // Should remember "Alice" from turn 1
    ];

    for (i, user_msg) in turns.iter().enumerate() {
        print!(
            "\n[Turn {}] User: {}\n[Turn {}] Assistant: ",
            i + 1,
            user_msg,
            i + 1
        );

        // Each run() call:
        // 1. Loads history from ThreadRunStore for this thread
        // 2. Appends the new user message
        // 3. Runs inference
        // 4. Checkpoints messages + run record back to store
        let request =
            RunRequest::new(thread_id, vec![Message::user(*user_msg)]).with_agent_id("chat");

        match runtime.run(request, &ConsoleSink).await {
            Ok((_handle, result)) => {
                print!(" [{:?}]", result.termination);
            }
            Err(e) => {
                print!(" [ERROR: {e}]");
            }
        }
    }

    print!("\n\n=== Multi-turn test complete ===\n");
}
