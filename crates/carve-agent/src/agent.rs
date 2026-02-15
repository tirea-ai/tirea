//! Agent struct and subagent system.
//!
//! Provides [`Agent`] as the primary interface for running agent loops,
//! creating subagents, and managing background agent tasks.

use crate::r#loop::{
    run_loop_stream, AgentDefinition, AgentLoopError, ChannelStateCommitter, RunContext,
};
use crate::stream::AgentEvent;
use crate::thread::Thread;
use crate::thread_store::{ThreadStore, ThreadStoreError};
use crate::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use genai::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Filter tools based on an `AgentDefinition`'s allowed/excluded lists.
pub fn filter_tools(
    tools: &HashMap<String, Arc<dyn Tool>>,
    definition: &AgentDefinition,
) -> HashMap<String, Arc<dyn Tool>> {
    tools
        .iter()
        .filter(|(_, tool)| {
            crate::tool_filter::is_tool_allowed_by_definition(&tool.descriptor().id, definition)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Result returned when a subagent completes.
#[derive(Debug, Clone)]
pub struct SubAgentResult {
    /// The final session state.
    pub thread: Thread,
    /// The final text response.
    pub response: String,
}

/// Handle to a background subagent task.
pub struct SubAgentHandle {
    /// The agent definition id.
    pub agent_id: String,
    thread_id: String,
    join: JoinHandle<Result<SubAgentResult, AgentLoopError>>,
    cancel: CancellationToken,
}

impl SubAgentHandle {
    /// Wait for the subagent to complete.
    pub async fn wait(self) -> Result<SubAgentResult, AgentLoopError> {
        self.join
            .await
            .map_err(|e| AgentLoopError::LlmError(format!("Task join error: {}", e)))?
    }

    /// Check if the subagent has finished.
    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }

    /// Abort the subagent.
    pub fn abort(&self) {
        self.cancel.cancel();
        self.join.abort();
    }

    /// Get the session id of the subagent.
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }
}

/// Primary agent interface wrapping definition, client, tools, and thread store.
pub struct Agent {
    definition: AgentDefinition,
    client: Client,
    tools: HashMap<String, Arc<dyn Tool>>,
    thread_store: Option<Arc<dyn ThreadStore>>,
}

impl Agent {
    /// Create a new agent with the given definition and client.
    pub fn new(definition: AgentDefinition, client: Client) -> Self {
        Self {
            definition,
            client,
            tools: HashMap::new(),
            thread_store: None,
        }
    }

    /// Add a tool.
    pub fn with_tool(mut self, tool: impl Tool + 'static) -> Self {
        let name = tool.descriptor().id.clone();
        self.tools.insert(name, Arc::new(tool));
        self
    }

    /// Add tools from a map.
    pub fn with_tools(mut self, tools: HashMap<String, Arc<dyn Tool>>) -> Self {
        self.tools.extend(tools);
        self
    }

    /// Set thread store backend.
    pub fn with_thread_store(mut self, thread_store: Arc<dyn ThreadStore>) -> Self {
        self.thread_store = Some(thread_store);
        self
    }

    /// Get the definition.
    pub fn definition(&self) -> &AgentDefinition {
        &self.definition
    }

    /// Run the agent loop, returning a stream of events.
    pub fn run(&self, thread: Thread) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        let filtered = filter_tools(&self.tools, &self.definition);
        run_loop_stream(
            self.client.clone(),
            self.definition.clone(),
            thread,
            filtered,
            RunContext::default(),
        )
    }

    /// Create a child agent inheriting client and thread store, with filtered tools.
    pub fn subagent(&self, definition: AgentDefinition) -> Agent {
        let filtered = filter_tools(&self.tools, &definition);
        Agent {
            definition,
            client: self.client.clone(),
            tools: filtered,
            thread_store: self.thread_store.clone(),
        }
    }

    /// Run a subagent synchronously, returning a stream of events.
    pub fn run_subagent(
        &self,
        prompt: impl Into<String>,
        resume_thread: Option<Thread>,
    ) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        let prompt = prompt.into();
        let thread = match resume_thread {
            Some(s) => s.with_message(crate::types::Message::user(&prompt)),
            None => {
                let id = format!("sub-{}-{}", self.definition.id, uuid_v7());
                Thread::new(id).with_message(crate::types::Message::user(prompt))
            }
        };
        self.run(thread)
    }

    /// Spawn a subagent in the background.
    pub fn spawn(&self, prompt: impl Into<String>) -> SubAgentHandle {
        let prompt = prompt.into();
        let thread_id = format!("sub-{}-{}", self.definition.id, uuid_v7());
        let thread =
            Thread::new(thread_id.clone()).with_message(crate::types::Message::user(&prompt));

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let agent_id = self.definition.id.clone();

        let filtered = filter_tools(&self.tools, &self.definition);
        let client = self.client.clone();
        let definition = self.definition.clone();
        let thread_store = self.thread_store.clone();

        let join = tokio::spawn(async move {
            let (checkpoint_tx, mut checkpoints) = tokio::sync::mpsc::unbounded_channel();
            let run_ctx = RunContext::default()
                .with_state_committer(Arc::new(ChannelStateCommitter::new(checkpoint_tx)));
            let mut stream = run_loop_stream(client, definition, thread.clone(), filtered, run_ctx);
            let mut last_response = String::new();
            let mut final_thread = thread.clone();
            let mut checkpoints_open = true;

            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        return Err(AgentLoopError::LlmError("Cancelled".to_string()));
                    }
                    checkpoint = checkpoints.recv(), if checkpoints_open => {
                        match checkpoint {
                            Some(delta) => delta.apply_to(&mut final_thread),
                            None => checkpoints_open = false,
                        }
                    }
                    event = stream.next() => {
                        match event {
                            Some(AgentEvent::RunFinish { result, .. }) => {
                                last_response = AgentEvent::extract_response(&result);
                            }
                            Some(AgentEvent::Error { message }) => {
                                return Err(AgentLoopError::LlmError(message));
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }
                }
            }

            while checkpoints_open {
                match checkpoints.recv().await {
                    Some(delta) => delta.apply_to(&mut final_thread),
                    None => checkpoints_open = false,
                }
            }

            // Save session if thread store is available.
            if let Some(ref thread_store) = thread_store {
                let _ = thread_store.save(&final_thread).await;
            }

            Ok(SubAgentResult {
                thread: final_thread,
                response: last_response,
            })
        });

        SubAgentHandle {
            agent_id,
            thread_id,
            join,
            cancel,
        }
    }

    /// Resume a previously saved session.
    pub async fn resume(
        &self,
        thread_id: &str,
        prompt: impl Into<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = AgentEvent> + Send>>, ThreadStoreError> {
        let thread_store = self.thread_store.as_ref().ok_or_else(|| {
            ThreadStoreError::Io(std::io::Error::other("No thread store configured"))
        })?;

        let thread = thread_store
            .load_thread(thread_id)
            .await?
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_string()))?;

        let thread = thread.with_message(crate::types::Message::user(prompt.into()));
        Ok(self.run(thread))
    }
}

/// Tool that delegates execution to a subagent.
pub struct SubAgentTool {
    agent: Arc<Agent>,
    name: String,
    description: String,
}

impl SubAgentTool {
    /// Create a new subagent tool.
    pub fn new(agent: Agent, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            agent: Arc::new(agent),
            name: name.into(),
            description: description.into(),
        }
    }
}

#[async_trait]
impl Tool for SubAgentTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(&self.name, &self.name, &self.description).with_parameters(json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The prompt/instruction for the subagent"
                },
                "resume_thread_id": {
                    "type": "string",
                    "description": "Optional thread ID to resume from"
                }
            },
            "required": ["prompt"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &carve_state::Context<'_>,
    ) -> Result<ToolResult, ToolError> {
        let prompt = args["prompt"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'prompt' argument".to_string()))?;

        let resume_id = args["resume_thread_id"].as_str();

        let mut stream = if let Some(id) = resume_id {
            match self.agent.resume(id, prompt).await {
                Ok(s) => s,
                Err(e) => {
                    return Ok(ToolResult::error(
                        &self.name,
                        format!("Resume error: {}", e),
                    ));
                }
            }
        } else {
            self.agent.run_subagent(prompt, None)
        };

        // Consume stream to get final response
        let mut response = String::new();
        let mut thread_id = String::new();
        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::RunStart { thread_id: tid, .. } => {
                    thread_id = tid;
                }
                AgentEvent::RunFinish { result, .. } => {
                    response = AgentEvent::extract_response(&result);
                    break;
                }
                AgentEvent::Error { message } => {
                    return Ok(ToolResult::error(&self.name, message));
                }
                _ => {}
            }
        }

        Ok(ToolResult::success(
            &self.name,
            json!({
                "response": response,
                "thread_id": thread_id,
            }),
        ))
    }
}

/// Generate a RFC4122 UUID v7 string without hyphens.
pub(crate) fn uuid_v7() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#loop::AgentDefinition;
    use crate::thread_store::{ThreadReader, ThreadWriter};
    use carve_state::Context;

    #[test]
    fn test_filter_tools_no_filters() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("a".into(), Arc::new(DummyTool("a")));
        tools.insert("b".into(), Arc::new(DummyTool("b")));

        let def = AgentDefinition::default();
        let filtered = filter_tools(&tools, &def);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_tools_allowed() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("a".into(), Arc::new(DummyTool("a")));
        tools.insert("b".into(), Arc::new(DummyTool("b")));
        tools.insert("c".into(), Arc::new(DummyTool("c")));

        let def = AgentDefinition::default().with_allowed_tools(vec!["a".into(), "b".into()]);
        let filtered = filter_tools(&tools, &def);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains_key("a"));
        assert!(filtered.contains_key("b"));
        assert!(!filtered.contains_key("c"));
    }

    #[test]
    fn test_filter_tools_excluded() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("a".into(), Arc::new(DummyTool("a")));
        tools.insert("b".into(), Arc::new(DummyTool("b")));

        let def = AgentDefinition::default().with_excluded_tools(vec!["b".into()]);
        let filtered = filter_tools(&tools, &def);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("a"));
    }

    #[test]
    fn test_filter_tools_allowed_and_excluded() {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("a".into(), Arc::new(DummyTool("a")));
        tools.insert("b".into(), Arc::new(DummyTool("b")));
        tools.insert("c".into(), Arc::new(DummyTool("c")));

        let def = AgentDefinition::default()
            .with_allowed_tools(vec!["a".into(), "b".into()])
            .with_excluded_tools(vec!["b".into()]);
        let filtered = filter_tools(&tools, &def);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("a"));
    }

    #[test]
    fn test_agent_definition_with_id() {
        let def = AgentDefinition::with_id("researcher", "gpt-4o-mini")
            .with_system_prompt("Research things")
            .with_max_rounds(5)
            .with_allowed_tools(vec!["search".into()]);

        assert_eq!(def.id, "researcher");
        assert_eq!(def.model, "gpt-4o-mini");
        assert_eq!(def.max_rounds, 5);
        assert_eq!(def.allowed_tools, Some(vec!["search".into()]));
    }

    #[test]
    fn test_agent_subagent_inherits_client() {
        let def = AgentDefinition::default();
        let client = Client::default();
        let agent = Agent::new(def, client)
            .with_tool(DummyTool("a"))
            .with_tool(DummyTool("b"));

        let sub_def =
            AgentDefinition::with_id("sub", "gpt-4o-mini").with_allowed_tools(vec!["a".into()]);
        let sub = agent.subagent(sub_def);

        assert_eq!(sub.tools.len(), 1);
        assert!(sub.tools.contains_key("a"));
        assert_eq!(sub.definition.id, "sub");
    }

    #[tokio::test]
    async fn test_subagent_handle_thread_id() {
        let cancel = CancellationToken::new();
        let handle = SubAgentHandle {
            agent_id: "test".into(),
            thread_id: "thread-123".into(),
            join: tokio::spawn(async {
                Ok(SubAgentResult {
                    thread: Thread::new("s"),
                    response: "done".into(),
                })
            }),
            cancel,
        };
        assert_eq!(handle.thread_id(), "thread-123");
        assert_eq!(handle.agent_id, "test");
        assert!(!handle.is_finished() || handle.is_finished()); // just test the method exists
        let result = handle.wait().await.unwrap();
        assert_eq!(result.response, "done");
    }

    // ========================================================================
    // Test helpers
    // ========================================================================

    struct DummyTool(&'static str);

    #[async_trait]
    impl Tool for DummyTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(self.0, self.0, "dummy")
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &carve_state::Context<'_>,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success(self.0, json!({"ok": true})))
        }
    }

    fn make_tools(names: &[&'static str]) -> HashMap<String, Arc<dyn Tool>> {
        names
            .iter()
            .map(|n| (n.to_string(), Arc::new(DummyTool(n)) as Arc<dyn Tool>))
            .collect()
    }

    fn make_agent_with_tools(names: &[&'static str]) -> Agent {
        let def = AgentDefinition::default();
        let client = Client::default();
        let mut agent = Agent::new(def, client);
        agent.tools = make_tools(names);
        agent
    }

    // ========================================================================
    // AgentDefinition builder tests
    // ========================================================================

    #[test]
    fn test_agent_definition_default() {
        let def = AgentDefinition::default();
        assert_eq!(def.id, "default");
        assert_eq!(def.model, "gpt-4o-mini");
        assert_eq!(def.max_rounds, 10);
        assert!(def.parallel_tools);
        assert!(def.system_prompt.is_empty());
        assert!(def.allowed_tools.is_none());
        assert!(def.excluded_tools.is_none());
        assert!(def.allowed_skills.is_none());
        assert!(def.excluded_skills.is_none());
        assert!(def.allowed_agents.is_none());
        assert!(def.excluded_agents.is_none());
        assert!(def.chat_options.is_some());
        assert!(def.plugins.is_empty());
    }

    #[test]
    fn test_agent_definition_new_sets_model() {
        let def = AgentDefinition::new("claude-3");
        assert_eq!(def.id, "default");
        assert_eq!(def.model, "claude-3");
    }

    #[test]
    fn test_agent_definition_full_builder_chain() {
        let def = AgentDefinition::with_id("planner", "gpt-4")
            .with_system_prompt("Plan things")
            .with_max_rounds(3)
            .with_parallel_tools(false)
            .with_allowed_tools(vec!["search".into(), "read".into()])
            .with_excluded_tools(vec!["dangerous".into()])
            .with_allowed_skills(vec!["skill-a".into()])
            .with_excluded_skills(vec!["skill-b".into()])
            .with_allowed_agents(vec!["writer".into()])
            .with_excluded_agents(vec!["reviewer".into()]);

        assert_eq!(def.id, "planner");
        assert_eq!(def.model, "gpt-4");
        assert_eq!(def.system_prompt, "Plan things");
        assert_eq!(def.max_rounds, 3);
        assert!(!def.parallel_tools);
        assert_eq!(
            def.allowed_tools,
            Some(vec!["search".into(), "read".into()])
        );
        assert_eq!(def.excluded_tools, Some(vec!["dangerous".into()]));
        assert_eq!(def.allowed_skills, Some(vec!["skill-a".into()]));
        assert_eq!(def.excluded_skills, Some(vec!["skill-b".into()]));
        assert_eq!(def.allowed_agents, Some(vec!["writer".into()]));
        assert_eq!(def.excluded_agents, Some(vec!["reviewer".into()]));
    }

    #[test]
    fn test_agent_definition_debug_format() {
        let def = AgentDefinition::with_id("test-agent", "gpt-4")
            .with_system_prompt("Hello")
            .with_allowed_tools(vec!["a".into()]);
        let debug = format!("{:?}", def);
        assert!(debug.contains("AgentDefinition"));
        assert!(debug.contains("test-agent"));
        assert!(debug.contains("gpt-4"));
        assert!(debug.contains("allowed_tools"));
    }

    #[test]
    fn test_agent_definition_clone() {
        let def = AgentDefinition::with_id("orig", "gpt-4").with_allowed_tools(vec!["a".into()]);
        let cloned = def.clone();
        assert_eq!(cloned.id, "orig");
        assert_eq!(cloned.allowed_tools, Some(vec!["a".into()]));
    }

    // ========================================================================
    // filter_tools edge cases
    // ========================================================================

    #[test]
    fn test_filter_tools_empty_tools() {
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let def = AgentDefinition::default().with_allowed_tools(vec!["a".into()]);
        let filtered = filter_tools(&tools, &def);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_tools_empty_allowed_list() {
        let tools = make_tools(&["a", "b"]);
        let def = AgentDefinition::default().with_allowed_tools(vec![]);
        let filtered = filter_tools(&tools, &def);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_tools_empty_excluded_list() {
        let tools = make_tools(&["a", "b"]);
        let def = AgentDefinition::default().with_excluded_tools(vec![]);
        let filtered = filter_tools(&tools, &def);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_tools_allowed_nonexistent_tool() {
        let tools = make_tools(&["a"]);
        let def =
            AgentDefinition::default().with_allowed_tools(vec!["a".into(), "nonexistent".into()]);
        let filtered = filter_tools(&tools, &def);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("a"));
    }

    #[test]
    fn test_filter_tools_excluded_nonexistent_tool() {
        let tools = make_tools(&["a", "b"]);
        let def = AgentDefinition::default().with_excluded_tools(vec!["nonexistent".into()]);
        let filtered = filter_tools(&tools, &def);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_tools_exclude_all() {
        let tools = make_tools(&["a", "b"]);
        let def = AgentDefinition::default().with_excluded_tools(vec!["a".into(), "b".into()]);
        let filtered = filter_tools(&tools, &def);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_tools_preserves_arc_identity() {
        let tool_a: Arc<dyn Tool> = Arc::new(DummyTool("a"));
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("a".into(), tool_a.clone());

        let def = AgentDefinition::default();
        let filtered = filter_tools(&tools, &def);
        assert!(Arc::ptr_eq(&tool_a, filtered.get("a").unwrap()));
    }

    // ========================================================================
    // Agent construction tests
    // ========================================================================

    #[test]
    fn test_agent_new_has_no_tools() {
        let agent = Agent::new(AgentDefinition::default(), Client::default());
        assert!(agent.tools.is_empty());
        assert!(agent.thread_store.is_none());
    }

    #[test]
    fn test_agent_with_tool_adds_tool() {
        let agent = Agent::new(AgentDefinition::default(), Client::default())
            .with_tool(DummyTool("alpha"))
            .with_tool(DummyTool("beta"));
        assert_eq!(agent.tools.len(), 2);
        assert!(agent.tools.contains_key("alpha"));
        assert!(agent.tools.contains_key("beta"));
    }

    #[test]
    fn test_agent_with_tools_from_map() {
        let tools = make_tools(&["x", "y"]);
        let agent = Agent::new(AgentDefinition::default(), Client::default()).with_tools(tools);
        assert_eq!(agent.tools.len(), 2);
    }

    #[test]
    fn test_agent_with_tools_merges() {
        let agent = Agent::new(AgentDefinition::default(), Client::default())
            .with_tool(DummyTool("a"))
            .with_tools(make_tools(&["b", "c"]));
        assert_eq!(agent.tools.len(), 3);
    }

    #[test]
    fn test_agent_with_storage() {
        let thread_store: Arc<dyn ThreadStore> =
            Arc::new(carve_thread_store_adapters::MemoryStore::new());
        let agent = Agent::new(AgentDefinition::default(), Client::default())
            .with_thread_store(thread_store);
        assert!(agent.thread_store.is_some());
    }

    #[test]
    fn test_agent_definition_accessor() {
        let def = AgentDefinition::with_id("my-agent", "gpt-4");
        let agent = Agent::new(def, Client::default());
        assert_eq!(agent.definition().id, "my-agent");
        assert_eq!(agent.definition().model, "gpt-4");
    }

    // ========================================================================
    // Agent.subagent() tests
    // ========================================================================

    #[test]
    fn test_subagent_filters_tools_by_allowed() {
        let agent = make_agent_with_tools(&["search", "write", "delete"]);
        let sub_def = AgentDefinition::with_id("reader", "gpt-4o-mini")
            .with_allowed_tools(vec!["search".into()]);
        let sub = agent.subagent(sub_def);

        assert_eq!(sub.tools.len(), 1);
        assert!(sub.tools.contains_key("search"));
    }

    #[test]
    fn test_subagent_filters_tools_by_excluded() {
        let agent = make_agent_with_tools(&["search", "write", "delete"]);
        let sub_def = AgentDefinition::with_id("safe", "gpt-4o-mini")
            .with_excluded_tools(vec!["delete".into()]);
        let sub = agent.subagent(sub_def);

        assert_eq!(sub.tools.len(), 2);
        assert!(sub.tools.contains_key("search"));
        assert!(sub.tools.contains_key("write"));
        assert!(!sub.tools.contains_key("delete"));
    }

    #[test]
    fn test_subagent_no_filter_gets_all_parent_tools() {
        let agent = make_agent_with_tools(&["a", "b", "c"]);
        let sub_def = AgentDefinition::with_id("sub", "gpt-4o-mini");
        let sub = agent.subagent(sub_def);
        assert_eq!(sub.tools.len(), 3);
    }

    #[test]
    fn test_subagent_inherits_storage() {
        let thread_store: Arc<dyn ThreadStore> =
            Arc::new(carve_thread_store_adapters::MemoryStore::new());
        let def = AgentDefinition::default();
        let agent = Agent::new(def, Client::default()).with_thread_store(thread_store.clone());

        let sub = agent.subagent(AgentDefinition::with_id("sub", "gpt-4o-mini"));
        assert!(sub.thread_store.is_some());
    }

    #[test]
    fn test_subagent_uses_own_definition() {
        let agent = make_agent_with_tools(&["a"]);
        let sub_def = AgentDefinition::with_id("custom", "claude-3")
            .with_system_prompt("Custom prompt")
            .with_max_rounds(3);
        let sub = agent.subagent(sub_def);

        assert_eq!(sub.definition.id, "custom");
        assert_eq!(sub.definition.model, "claude-3");
        assert_eq!(sub.definition.system_prompt, "Custom prompt");
        assert_eq!(sub.definition.max_rounds, 3);
    }

    #[test]
    fn test_nested_subagent() {
        let agent = make_agent_with_tools(&["a", "b", "c"]);
        let sub1 = agent.subagent(
            AgentDefinition::with_id("sub1", "gpt-4o-mini")
                .with_allowed_tools(vec!["a".into(), "b".into()]),
        );
        assert_eq!(sub1.tools.len(), 2);

        // Sub-sub agent further restricts
        let sub2 = sub1.subagent(
            AgentDefinition::with_id("sub2", "gpt-4o-mini").with_allowed_tools(vec!["a".into()]),
        );
        assert_eq!(sub2.tools.len(), 1);
        assert!(sub2.tools.contains_key("a"));
    }

    // ========================================================================
    // SubAgentHandle tests
    // ========================================================================

    #[tokio::test]
    async fn test_subagent_handle_wait_returns_result() {
        let handle = SubAgentHandle {
            agent_id: "a".into(),
            thread_id: "s".into(),
            join: tokio::spawn(async {
                Ok(SubAgentResult {
                    thread: Thread::new("s"),
                    response: "hello".into(),
                })
            }),
            cancel: CancellationToken::new(),
        };

        let result = handle.wait().await.unwrap();
        assert_eq!(result.response, "hello");
        assert_eq!(result.thread.id, "s");
    }

    #[tokio::test]
    async fn test_subagent_handle_wait_propagates_error() {
        let handle = SubAgentHandle {
            agent_id: "a".into(),
            thread_id: "s".into(),
            join: tokio::spawn(async {
                Err(AgentLoopError::Stopped {
                    thread: Box::new(Thread::new("s")),
                    reason: crate::stop::StopReason::MaxRoundsReached,
                })
            }),
            cancel: CancellationToken::new(),
        };

        let err = handle.wait().await.unwrap_err();
        match err {
            AgentLoopError::Stopped { reason, .. } => {
                assert_eq!(reason, crate::stop::StopReason::MaxRoundsReached);
            }
            other => panic!("Expected Stopped(MaxRoundsReached), got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_subagent_handle_abort() {
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let handle = SubAgentHandle {
            agent_id: "a".into(),
            thread_id: "s".into(),
            join: tokio::spawn(async move {
                cancel_clone.cancelled().await;
                Err(AgentLoopError::LlmError("Cancelled".into()))
            }),
            cancel,
        };

        handle.abort();
        // After abort, wait should return a JoinError (task was aborted)
        let result = handle.join.await;
        assert!(result.is_err()); // JoinError from abort
    }

    #[tokio::test]
    async fn test_subagent_handle_is_finished() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let handle = SubAgentHandle {
            agent_id: "a".into(),
            thread_id: "s".into(),
            join: tokio::spawn(async move {
                rx.await.ok();
                Ok(SubAgentResult {
                    thread: Thread::new("s"),
                    response: "done".into(),
                })
            }),
            cancel: CancellationToken::new(),
        };

        // Not finished yet
        assert!(!handle.is_finished());
        tx.send(()).ok();
        // Give the task a moment to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert!(handle.is_finished());
    }

    // ========================================================================
    // SubAgentResult tests
    // ========================================================================

    #[test]
    fn test_subagent_result_clone() {
        let result = SubAgentResult {
            thread: Thread::new("s1"),
            response: "answer".into(),
        };
        let cloned = result.clone();
        assert_eq!(cloned.thread.id, "s1");
        assert_eq!(cloned.response, "answer");
    }

    #[test]
    fn test_subagent_result_debug() {
        let result = SubAgentResult {
            thread: Thread::new("s1"),
            response: "test".into(),
        };
        let debug = format!("{:?}", result);
        assert!(debug.contains("SubAgentResult"));
    }

    // ========================================================================
    // SubAgentTool tests
    // ========================================================================

    #[test]
    fn test_subagent_tool_descriptor() {
        let agent = Agent::new(AgentDefinition::default(), Client::default());
        let tool = SubAgentTool::new(agent, "researcher", "Researches topics");
        let desc = tool.descriptor();

        assert_eq!(desc.id, "researcher");
        assert_eq!(desc.description, "Researches topics");
        // Check parameters schema
        let params = &desc.parameters;
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["prompt"].is_object());
        assert!(params["properties"]["resume_thread_id"].is_object());
        let required = params["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "prompt");
    }

    #[tokio::test]
    async fn test_subagent_tool_execute_missing_prompt() {
        let agent = Agent::new(AgentDefinition::default(), Client::default());
        let tool = SubAgentTool::new(agent, "test", "test tool");
        let state = json!({});
        let ctx = carve_state::Context::new(&state, "call_1", "test");

        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
        match result.err().unwrap() {
            ToolError::InvalidArguments(msg) => {
                assert!(msg.contains("prompt"));
            }
            other => panic!("Expected InvalidArguments, got: {:?}", other),
        }
    }

    // ========================================================================
    // Agent.resume() tests
    // ========================================================================

    #[tokio::test]
    async fn test_resume_without_storage_returns_error() {
        let agent = Agent::new(AgentDefinition::default(), Client::default());
        match agent.resume("some-thread", "continue").await {
            Err(ThreadStoreError::Io(e)) => {
                assert!(e.to_string().contains("No thread store configured"));
            }
            Err(other) => panic!("Expected Io error, got: {:?}", other),
            Ok(_) => panic!("Expected error"),
        }
    }

    #[tokio::test]
    async fn test_resume_with_nonexistent_session() {
        let thread_store: Arc<dyn ThreadStore> =
            Arc::new(carve_thread_store_adapters::MemoryStore::new());
        let agent = Agent::new(AgentDefinition::default(), Client::default())
            .with_thread_store(thread_store);
        match agent.resume("nonexistent", "continue").await {
            Err(ThreadStoreError::NotFound(id)) => {
                assert_eq!(id, "nonexistent");
            }
            Err(other) => panic!("Expected NotFound, got: {:?}", other),
            Ok(_) => panic!("Expected error"),
        }
    }

    #[tokio::test]
    async fn test_resume_with_existing_session() {
        let thread_store = Arc::new(carve_thread_store_adapters::MemoryStore::new());
        let thread = Thread::new("saved-thread")
            .with_message(crate::types::Message::user("original prompt"));
        thread_store.save(&thread).await.unwrap();

        let storage_arc: Arc<dyn ThreadStore> = thread_store;
        let agent = Agent::new(AgentDefinition::default(), Client::default())
            .with_thread_store(storage_arc);

        // resume returns a stream (will fail on LLM call, but that's ok - we just test it loads)
        let result = agent.resume("saved-thread", "follow up").await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Agent.spawn() tests
    // ========================================================================

    #[tokio::test]
    async fn test_spawn_creates_handle_with_correct_agent_id() {
        let def = AgentDefinition::with_id("spawner", "gpt-4o-mini");
        let agent = Agent::new(def, Client::default());
        let handle = agent.spawn("do something");

        assert_eq!(handle.agent_id, "spawner");
        assert!(handle.thread_id().starts_with("sub-spawner-"));
        handle.abort();
    }

    #[tokio::test]
    async fn test_spawn_session_id_suffix_is_rfc4122_uuid_v7() {
        let def = AgentDefinition::with_id("spawner", "gpt-4o-mini");
        let agent = Agent::new(def, Client::default());
        let handle = agent.spawn("do something");

        let suffix = handle
            .thread_id()
            .rsplit_once('-')
            .map(|(_, right)| right)
            .unwrap_or_else(|| {
                panic!("thread id must contain uuid suffix: {}", handle.thread_id())
            });
        let parsed = uuid::Uuid::parse_str(suffix)
            .unwrap_or_else(|_| panic!("thread suffix must be parseable UUID, got: {suffix}"));
        assert_eq!(
            parsed.get_variant(),
            uuid::Variant::RFC4122,
            "thread suffix must be RFC4122 UUID, got: {suffix}"
        );
        assert_eq!(
            parsed.get_version_num(),
            7,
            "thread suffix must be version 7 UUID, got: {suffix}"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn test_spawn_unique_session_ids() {
        let agent = Agent::new(AgentDefinition::default(), Client::default());
        let h1 = agent.spawn("task 1");
        // tiny sleep to ensure different timestamp
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        let h2 = agent.spawn("task 2");

        assert_ne!(h1.thread_id(), h2.thread_id());
        h1.abort();
        h2.abort();
    }

    // ========================================================================
    // Agent.run_subagent() session construction tests
    // ========================================================================

    #[test]
    fn test_run_subagent_creates_session_with_prompt() {
        let agent = make_agent_with_tools(&["a"]);
        // We can't easily inspect the session inside the stream,
        // but we can verify it returns a stream (Pin<Box<dyn Stream>>)
        let _stream = agent.run_subagent("test prompt", None);
        // stream is valid; it'll fail on LLM call but construction is correct
    }

    #[test]
    fn test_run_subagent_with_resume_thread() {
        let agent = make_agent_with_tools(&["a"]);
        let existing = Thread::new("existing")
            .with_message(crate::types::Message::user("original"))
            .with_message(crate::types::Message::assistant("response"));

        let _stream = agent.run_subagent("follow up", Some(existing));
    }

    // ========================================================================
    // Integration: definition-level tool filtering in loop
    // ========================================================================

    #[test]
    fn test_agent_definition_allowed_tools_type_alias_compat() {
        // Verify AgentConfig type alias works
        use crate::r#loop::AgentConfig;
        let config: AgentConfig = AgentConfig::new("gpt-4").with_allowed_tools(vec!["a".into()]);
        assert_eq!(config.allowed_tools, Some(vec!["a".into()]));
    }

    // ========================================================================
    // uuid helper uniqueness
    // ========================================================================

    #[test]
    fn test_uuid_v7_not_empty() {
        let id = uuid_v7();
        assert!(!id.is_empty());
        assert_eq!(id.len(), 32);
    }

    #[test]
    fn test_uuid_v7_generates_different_values() {
        let id1 = uuid_v7();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let id2 = uuid_v7();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_uuid_v7_is_rfc4122_version7_uuid() {
        for _ in 0..16 {
            let id = uuid_v7();
            let parsed = uuid::Uuid::parse_str(&id)
                .unwrap_or_else(|_| panic!("uuid_v7() must return parseable UUID, got: {id}"));
            assert_eq!(
                parsed.get_variant(),
                uuid::Variant::RFC4122,
                "uuid_v7() must be RFC4122 variant, got: {id}"
            );
            assert_eq!(
                parsed.get_version_num(),
                7,
                "uuid_v7() must be version 7, got: {id}"
            );
        }
    }

    // ========================================================================
    // Plugins for integration tests (skip LLM calls, record state)
    // ========================================================================

    use crate::phase::{Phase, StepContext};
    use crate::plugin::AgentPlugin;
    use std::sync::Mutex;

    /// Plugin that skips LLM inference — makes run_loop_stream return immediately.
    struct SkipInferencePlugin;

    #[async_trait]
    impl AgentPlugin for SkipInferencePlugin {
        fn id(&self) -> &str {
            "skip_inference"
        }
        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    /// Plugin that writes a marker into session state during SessionEnd.
    struct ThreadEndStatePatchPlugin;

    #[async_trait]
    impl AgentPlugin for ThreadEndStatePatchPlugin {
        fn id(&self) -> &str {
            "session_end_state_patch"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
            if phase != Phase::SessionEnd {
                return;
            }
            let patch = carve_state::TrackedPatch::new(carve_state::Patch::new().with_op(
                carve_state::Op::set(carve_state::path!("spawn", "finished"), json!(true)),
            ))
            .with_source("test:session_end_state_patch");
            step.pending_patches.push(patch);
        }
    }

    /// Plugin that records the tool IDs visible during BeforeInference.
    struct ToolRecorderPlugin {
        recorded: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl ToolRecorderPlugin {
        fn new() -> (Self, Arc<Mutex<Vec<Vec<String>>>>) {
            let recorded = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    recorded: recorded.clone(),
                },
                recorded,
            )
        }
    }

    #[async_trait]
    impl AgentPlugin for ToolRecorderPlugin {
        fn id(&self) -> &str {
            "tool_recorder"
        }
        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
            if phase == Phase::BeforeInference {
                let tool_ids: Vec<String> = step.tools.iter().map(|t| t.id.clone()).collect();
                self.recorded.lock().unwrap().push(tool_ids);
                // Also skip inference so we don't need a real LLM
                step.skip_inference = true;
            }
        }
    }

    /// Collect all events from a stream until Done/Error/stream-end.
    async fn collect_events(
        stream: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    ) -> Vec<AgentEvent> {
        use futures::StreamExt;
        let mut events = Vec::new();
        let mut stream = stream;
        while let Some(event) = stream.next().await {
            let is_terminal = matches!(
                event,
                AgentEvent::RunFinish { .. } | AgentEvent::Error { .. }
            );
            events.push(event);
            if is_terminal {
                break;
            }
        }
        events
    }

    // ========================================================================
    // 1. Definition-level tool filtering in run_loop_stream / run_step
    // ========================================================================

    #[tokio::test]
    async fn test_run_loop_stream_allowed_tools_filters_descriptors() {
        let (recorder, recorded) = ToolRecorderPlugin::new();
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_allowed_tools(vec!["a".into(), "b".into()])
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let tools = make_tools(&["a", "b", "c", "d"]);
        let thread = Thread::new("test").with_message(crate::types::Message::user("hello"));

        let stream = run_loop_stream(Client::default(), def, thread, tools, RunContext::default());
        let _events = collect_events(stream).await;

        let snapshots = recorded.lock().unwrap();
        assert_eq!(snapshots.len(), 1); // one BeforeInference call
        let mut visible: Vec<String> = snapshots[0].clone();
        visible.sort();
        assert_eq!(visible, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn test_run_loop_stream_excluded_tools_filters_descriptors() {
        let (recorder, recorded) = ToolRecorderPlugin::new();
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_excluded_tools(vec!["c".into()])
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let tools = make_tools(&["a", "b", "c"]);
        let thread = Thread::new("test").with_message(crate::types::Message::user("hello"));

        let stream = run_loop_stream(Client::default(), def, thread, tools, RunContext::default());
        let _events = collect_events(stream).await;

        let snapshots = recorded.lock().unwrap();
        let mut visible: Vec<String> = snapshots[0].clone();
        visible.sort();
        assert_eq!(visible, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn test_run_loop_stream_allowed_and_excluded_combined() {
        let (recorder, recorded) = ToolRecorderPlugin::new();
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_allowed_tools(vec!["a".into(), "b".into(), "c".into()])
            .with_excluded_tools(vec!["b".into()])
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let tools = make_tools(&["a", "b", "c", "d"]);
        let thread = Thread::new("test").with_message(crate::types::Message::user("hello"));

        let stream = run_loop_stream(Client::default(), def, thread, tools, RunContext::default());
        let _events = collect_events(stream).await;

        let snapshots = recorded.lock().unwrap();
        let mut visible: Vec<String> = snapshots[0].clone();
        visible.sort();
        assert_eq!(visible, vec!["a".to_string(), "c".to_string()]);
    }

    #[tokio::test]
    async fn test_run_loop_stream_no_filter_sees_all_tools() {
        let (recorder, recorded) = ToolRecorderPlugin::new();
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let tools = make_tools(&["x", "y", "z"]);
        let thread = Thread::new("test").with_message(crate::types::Message::user("hello"));

        let stream = run_loop_stream(Client::default(), def, thread, tools, RunContext::default());
        let _events = collect_events(stream).await;

        let snapshots = recorded.lock().unwrap();
        let mut visible: Vec<String> = snapshots[0].clone();
        visible.sort();
        assert_eq!(
            visible,
            vec!["x".to_string(), "y".to_string(), "z".to_string()]
        );
    }

    #[tokio::test]
    async fn test_run_step_with_allowed_tools() {
        use crate::r#loop::run_step;

        let (recorder, recorded) = ToolRecorderPlugin::new();
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_allowed_tools(vec!["a".into()])
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let tools = make_tools(&["a", "b", "c"]);
        let thread = Thread::new("test").with_message(crate::types::Message::user("hi"));

        // run_step will skip inference → return empty result
        let result = run_step(&Client::default(), &def, thread, &tools).await;
        assert!(result.is_ok());

        let snapshots = recorded.lock().unwrap();
        assert_eq!(snapshots[0], vec!["a".to_string()]);
    }

    // ========================================================================
    // 2. SubAgentTool.execute happy path (with skip_inference)
    // ========================================================================

    #[tokio::test]
    async fn test_subagent_tool_execute_happy_path() {
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default());
        let tool = SubAgentTool::new(agent, "helper", "Helps with tasks");

        let state = json!({});
        let ctx = carve_state::Context::new(&state, "call_1", "test");
        let result = tool
            .execute(json!({"prompt": "do something"}), &ctx)
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.tool_name, "helper");
        // The response field should be empty because skip_inference yields Done { response: "" }
        assert_eq!(result.data["response"], "");
    }

    // ========================================================================
    // 3. SubAgentTool.execute with resume_thread_id
    // ========================================================================

    #[tokio::test]
    async fn test_subagent_tool_execute_with_resume_no_storage() {
        // SubAgentTool with no thread_store configured → resume should return error result
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default());
        let tool = SubAgentTool::new(agent, "helper", "Helps");

        let state = json!({});
        let ctx = carve_state::Context::new(&state, "call_1", "test");
        let result = tool
            .execute(
                json!({"prompt": "continue", "resume_thread_id": "old-thread"}),
                &ctx,
            )
            .await
            .unwrap();

        // Should return an error result (not a ToolError), because resume fails gracefully
        assert!(!result.is_success());
        let data_str = result.data.to_string();
        let msg_str = result.message.as_deref().unwrap_or("");
        assert!(
            msg_str.contains("Resume error") || data_str.contains("Resume error"),
            "Expected resume error, got message={:?} data={}",
            msg_str,
            data_str
        );
    }

    #[tokio::test]
    async fn test_subagent_tool_execute_with_resume_thread_found() {
        let thread_store = Arc::new(carve_thread_store_adapters::MemoryStore::new());
        let saved_thread = Thread::new("old-thread")
            .with_message(crate::types::Message::user("first question"))
            .with_message(crate::types::Message::assistant("first answer"));
        thread_store.save(&saved_thread).await.unwrap();

        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default())
            .with_thread_store(thread_store as Arc<dyn ThreadStore>);
        let tool = SubAgentTool::new(agent, "helper", "Helps");

        let state = json!({});
        let ctx = carve_state::Context::new(&state, "call_1", "test");
        let result = tool
            .execute(
                json!({"prompt": "continue", "resume_thread_id": "old-thread"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.data["response"], "");
    }

    // ========================================================================
    // 4. Agent.spawn() with thread_store saves session
    // ========================================================================

    #[tokio::test]
    async fn test_spawn_saves_session_to_storage() {
        let thread_store = Arc::new(carve_thread_store_adapters::MemoryStore::new());
        let def = AgentDefinition::with_id("saver", "gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default())
            .with_thread_store(thread_store.clone() as Arc<dyn ThreadStore>);

        let handle = agent.spawn("save this");
        let thread_id = handle.thread_id().to_string();

        // Wait for completion
        let result = handle.wait().await.unwrap();
        assert_eq!(result.response, "");

        // Verify session was saved to thread store
        let saved = thread_store.load_thread(&thread_id).await.unwrap();
        assert!(
            saved.is_some(),
            "Thread '{}' should have been saved to thread store",
            thread_id
        );
        let saved = saved.unwrap();
        assert_eq!(saved.id, thread_id);
    }

    #[tokio::test]
    async fn test_spawn_returns_and_persists_final_thread_state() {
        let thread_store = Arc::new(carve_thread_store_adapters::MemoryStore::new());
        let def = AgentDefinition::with_id("spawn-final-state", "gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>)
            .with_plugin(Arc::new(ThreadEndStatePatchPlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default())
            .with_thread_store(thread_store.clone() as Arc<dyn ThreadStore>);

        let handle = agent.spawn("persist final thread");
        let thread_id = handle.thread_id().to_string();
        let result = handle.wait().await.unwrap();

        let result_state = result.thread.rebuild_state().unwrap();
        assert_eq!(result_state["spawn"]["finished"], true);

        let saved = thread_store
            .load_thread(&thread_id)
            .await
            .unwrap()
            .expect("thread exists");
        let saved_state = saved.rebuild_state().unwrap();
        assert_eq!(saved_state["spawn"]["finished"], true);
    }

    // ========================================================================
    // 5. Agent.spawn() cancellation within stream loop
    // ========================================================================

    /// Plugin that blocks during BeforeInference (simulating a long LLM call).
    struct BlockingPlugin {
        ready: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl AgentPlugin for BlockingPlugin {
        fn id(&self) -> &str {
            "blocking"
        }
        async fn on_phase(&self, phase: Phase, _step: &mut StepContext<'_>, _ctx: &Context<'_>) {
            if phase == Phase::BeforeInference {
                self.ready.notify_one();
                // Block forever (until task is cancelled)
                futures::future::pending::<()>().await;
            }
        }
    }

    #[tokio::test]
    async fn test_spawn_cancel_token_stops_stream_loop() {
        let ready = Arc::new(tokio::sync::Notify::new());
        let blocking_plugin = BlockingPlugin {
            ready: ready.clone(),
        };

        let def = AgentDefinition::with_id("blocker", "gpt-4o-mini")
            .with_plugin(Arc::new(blocking_plugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default());

        let handle = agent.spawn("long task");

        // Wait until the plugin is actually running
        ready.notified().await;

        // Task should not be finished yet
        assert!(!handle.is_finished());

        // Abort
        handle.abort();

        // Give it a moment
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Verify it is no longer running (join handle resolved)
        assert!(handle.join.is_finished());
    }

    // ========================================================================
    // 6. Agent.run() direct test
    // ========================================================================

    #[tokio::test]
    async fn test_agent_run_produces_run_finish_event() {
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default())
            .with_tool(DummyTool("a"))
            .with_tool(DummyTool("b"));

        let thread = Thread::new("test").with_message(crate::types::Message::user("hello"));

        let events = collect_events(agent.run(thread)).await;

        // With skip_inference, we should get RunStart and RunFinish events
        let finish_events: Vec<&AgentEvent> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::RunFinish { .. }))
            .collect();
        assert_eq!(finish_events.len(), 1);
        match &finish_events[0] {
            AgentEvent::RunFinish { result, .. } => {
                assert_eq!(AgentEvent::extract_response(result), "");
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn test_agent_run_with_allowed_tools_filters() {
        let (recorder, recorded) = ToolRecorderPlugin::new();
        // Note: ToolRecorderPlugin also skips inference
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_allowed_tools(vec!["a".into()])
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);
        let mut agent = Agent::new(def, Client::default());
        agent.tools = make_tools(&["a", "b", "c"]);

        let thread = Thread::new("test").with_message(crate::types::Message::user("hello"));
        let _events = collect_events(agent.run(thread)).await;

        // Agent.run() applies filter_tools on its tools map, then run_loop_stream
        // also applies definition-level filtering on descriptors.
        // Since Agent.run() uses filter_tools(), only "a" should be in the tools map.
        let snapshots = recorded.lock().unwrap();
        assert_eq!(snapshots[0], vec!["a".to_string()]);
    }

    // ========================================================================
    // 7. run_subagent stream events and session messages
    // ========================================================================

    #[tokio::test]
    async fn test_run_subagent_stream_events() {
        let def = AgentDefinition::with_id("sub", "gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default());

        let stream = agent.run_subagent("test prompt", None);
        let events = collect_events(stream).await;

        // Should have RunStart and RunFinish events
        let has_finish = events
            .iter()
            .any(|e| matches!(e, AgentEvent::RunFinish { .. }));
        assert!(has_finish, "Expected RunFinish event in stream");
    }

    #[tokio::test]
    async fn test_run_subagent_session_id_prefix() {
        // Verify the session id created by run_subagent follows expected format
        let def = AgentDefinition::with_id("researcher", "gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default());

        // We can't directly inspect thread, but the spawn method uses same format
        // Test via spawn which exposes session_id
        let handle = agent.spawn("test");
        assert!(
            handle.thread_id().starts_with("sub-researcher-"),
            "Thread ID '{}' should start with 'sub-researcher-'",
            handle.thread_id()
        );
        handle.abort();
    }

    #[tokio::test]
    async fn test_run_subagent_with_resume_adds_message() {
        let (recorder, recorded) = ToolRecorderPlugin::new();
        let def = AgentDefinition::with_id("sub", "gpt-4o-mini")
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default());

        let existing = Thread::new("old")
            .with_message(crate::types::Message::user("first"))
            .with_message(crate::types::Message::assistant("answer"));

        let stream = agent.run_subagent("follow up", Some(existing));
        let _events = collect_events(stream).await;

        // The ToolRecorderPlugin just records tools, but the key verification
        // is that run_subagent didn't panic and produced events.
        let snapshots = recorded.lock().unwrap();
        assert!(
            !snapshots.is_empty(),
            "BeforeInference should have been called"
        );
    }

    #[tokio::test]
    async fn test_run_subagent_stream_ends_with_run_finish_not_error() {
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let agent = Agent::new(def, Client::default()).with_tool(DummyTool("a"));

        let stream = agent.run_subagent("do task", None);
        let events = collect_events(stream).await;

        let last = events.last().expect("Should have at least one event");
        assert!(
            matches!(last, AgentEvent::RunFinish { .. }),
            "Last event should be RunFinish, got: {:?}",
            last
        );
    }

    // ========================================================================
    // Extra: Agent.run() with excluded_tools on definition level
    // ========================================================================

    #[tokio::test]
    async fn test_agent_run_excluded_tools_not_visible() {
        let (recorder, recorded) = ToolRecorderPlugin::new();
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_excluded_tools(vec!["dangerous".into()])
            .with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);
        let mut agent = Agent::new(def, Client::default());
        agent.tools = make_tools(&["safe", "dangerous"]);

        let thread = Thread::new("test").with_message(crate::types::Message::user("hi"));
        let _events = collect_events(agent.run(thread)).await;

        let snapshots = recorded.lock().unwrap();
        assert_eq!(snapshots[0], vec!["safe".to_string()]);
    }

    // ========================================================================
    // RunContext passthrough tests
    // ========================================================================

    #[tokio::test]
    async fn test_run_loop_stream_external_run_id_passthrough() {
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let mut thread = Thread::new("thread-42").with_message(crate::types::Message::user("hi"));
        let tools = HashMap::new();

        // Set run_id and parent_run_id on the session runtime
        let _ = thread.runtime.set("run_id", "external-run-id".to_string());
        let _ = thread
            .runtime
            .set("parent_run_id", "parent-run-id".to_string());

        let ctx = RunContext {
            cancellation_token: None,
            ..RunContext::default()
        };
        let events =
            collect_events(run_loop_stream(Client::default(), def, thread, tools, ctx)).await;

        // First event should be RunStart with our IDs
        let first = &events[0];
        if let AgentEvent::RunStart {
            thread_id,
            run_id,
            parent_run_id,
        } = first
        {
            assert_eq!(thread_id, "thread-42");
            assert_eq!(run_id, "external-run-id");
            assert_eq!(parent_run_id.as_deref(), Some("parent-run-id"));
        } else {
            panic!("Expected RunStart, got: {:?}", first);
        }

        // Last event should be RunFinish with the same run_id
        let last = events.last().unwrap();
        if let AgentEvent::RunFinish {
            thread_id, run_id, ..
        } = last
        {
            assert_eq!(thread_id, "thread-42");
            assert_eq!(run_id, "external-run-id");
        } else {
            panic!("Expected RunFinish, got: {:?}", last);
        }
    }

    #[tokio::test]
    async fn test_run_loop_stream_default_run_context_auto_generates() {
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let thread = Thread::new("test").with_message(crate::types::Message::user("hi"));
        let tools = HashMap::new();

        let events = collect_events(run_loop_stream(
            Client::default(),
            def,
            thread,
            tools,
            RunContext::default(),
        ))
        .await;

        let first = &events[0];
        if let AgentEvent::RunStart {
            run_id,
            parent_run_id,
            ..
        } = first
        {
            assert!(
                !run_id.is_empty(),
                "auto-generated run_id should be non-empty"
            );
            let parsed = uuid::Uuid::parse_str(run_id)
                .unwrap_or_else(|_| panic!("run_id must be parseable UUID, got: {run_id}"));
            assert_eq!(
                parsed.get_variant(),
                uuid::Variant::RFC4122,
                "run_id must be RFC4122 UUID, got: {run_id}"
            );
            assert_eq!(
                parsed.get_version_num(),
                7,
                "run_id must be version 7 UUID, got: {run_id}"
            );
            assert!(parent_run_id.is_none());
        } else {
            panic!("Expected RunStart, got: {:?}", first);
        }
    }

    #[tokio::test]
    async fn test_run_loop_stream_run_id_consistent_across_events() {
        let def = AgentDefinition::new("gpt-4o-mini")
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
        let mut thread = Thread::new("s1").with_message(crate::types::Message::user("hi"));
        let tools = HashMap::new();

        // Set run_id on the session runtime
        let _ = thread.runtime.set("run_id", "consistent-id".to_string());

        let ctx = RunContext {
            cancellation_token: None,
            ..RunContext::default()
        };
        let events =
            collect_events(run_loop_stream(Client::default(), def, thread, tools, ctx)).await;

        // Extract run_id from RunStart and RunFinish and verify they match
        let start_id = events
            .iter()
            .find_map(|e| {
                if let AgentEvent::RunStart { run_id, .. } = e {
                    Some(run_id.clone())
                } else {
                    None
                }
            })
            .expect("should have RunStart");

        let finish_id = events
            .iter()
            .find_map(|e| {
                if let AgentEvent::RunFinish { run_id, .. } = e {
                    Some(run_id.clone())
                } else {
                    None
                }
            })
            .expect("should have RunFinish");

        assert_eq!(start_id, "consistent-id");
        assert_eq!(finish_id, "consistent-id");
    }
}
