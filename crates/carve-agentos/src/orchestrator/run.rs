use super::*;
use crate::contracts::storage::VersionPrecondition;
use crate::runtime::loop_runner::{run_loop_stream, StaticStepToolProvider};

impl AgentOs {
    pub fn agent_state_store(&self) -> Option<&Arc<dyn AgentStateStore>> {
        self.agent_state_store.as_ref()
    }

    fn require_agent_state_store(&self) -> Result<&Arc<dyn AgentStateStore>, AgentOsRunError> {
        self.agent_state_store
            .as_ref()
            .ok_or(AgentOsRunError::AgentStateStoreNotConfigured)
    }

    fn generate_id() -> String {
        uuid::Uuid::now_v7().simple().to_string()
    }

    /// Load a thread from storage. Returns the thread and its version.
    /// If the thread does not exist, returns `None`.
    pub async fn load_agent_state(
        &self,
        id: &str,
    ) -> Result<Option<AgentStateHead>, AgentOsRunError> {
        let agent_state_store = self.require_agent_state_store()?;
        Ok(agent_state_store.load(id).await?)
    }

    /// Prepare a request for execution.
    ///
    /// This handles all deterministic pre-run logic:
    /// 1. Thread loading/creation from storage
    /// 2. Message deduplication and appending
    /// 3. Persisting pre-run state
    /// 4. Agent resolution and run-context creation
    /// 5. Append run-scoped plugins and tool registries from [`RunScope`]
    pub async fn prepare_run(
        &self,
        mut request: RunRequest,
        scope: RunScope,
    ) -> Result<PreparedRun, AgentOsRunError> {
        let agent_state_store = self.require_agent_state_store()?;

        // 0. Validate agent exists (fail fast before creating thread)
        self.validate_agent(&request.agent_id)?;

        let thread_id = request.thread_id.unwrap_or_else(Self::generate_id);
        let run_id = request.run_id.unwrap_or_else(Self::generate_id);
        let parent_run_id = request.parent_run_id.clone();

        // 1. Load or create thread
        //    If frontend sent a state snapshot, apply it:
        //    - New thread: used as initial state
        //    - Existing thread: replaces current state (persisted in UserMessage delta)
        let frontend_state = request.state.take();
        let mut state_snapshot_for_delta: Option<serde_json::Value> = None;
        let (mut thread, mut version) = match agent_state_store.load(&thread_id).await? {
            Some(head) => {
                let mut t = head.agent_state;
                if let Some(state) = frontend_state {
                    t.state = state.clone();
                    t.patches.clear();
                    state_snapshot_for_delta = Some(state);
                }
                (t, head.version)
            }
            None => {
                let thread = if let Some(state) = frontend_state {
                    Thread::with_initial_state(thread_id.clone(), state)
                } else {
                    Thread::new(thread_id.clone())
                };
                let committed = agent_state_store.create(&thread).await?;
                (thread, committed.version)
            }
        };

        // 2. Set resource_id on thread if provided
        if let Some(ref resource_id) = request.resource_id {
            thread.resource_id = Some(resource_id.clone());
        }

        // 3. Deduplicate and append inbound messages
        let deduped = Self::dedup_messages(&thread, request.messages);
        if !deduped.is_empty() {
            thread = thread.with_messages(deduped);
        }

        // 5. Persist pending changes (user messages + frontend state snapshot)
        let pending = thread.take_pending();
        if !pending.is_empty() || state_snapshot_for_delta.is_some() {
            let changeset = crate::contracts::ThreadChangeSet::from_parts(
                run_id.clone(),
                parent_run_id.clone(),
                CheckpointReason::UserMessage,
                pending.messages,
                pending.patches,
                state_snapshot_for_delta,
            );
            let committed = agent_state_store
                .append(&thread_id, &changeset, VersionPrecondition::Exact(version))
                .await?;
            version = committed.version;
        }
        thread.metadata.version = Some(version);

        // 4. Resolve static wiring.
        let (mut cfg, mut tools, thread, mut run_config) = self.resolve(&request.agent_id, thread)?;

        // Set run identity on the run_config
        let _ = run_config.set("run_id", run_id.clone());
        if let Some(parent) = parent_run_id.as_deref() {
            let _ = run_config.set("parent_run_id", parent.to_string());
        }

        // 5. Append run-scoped plugins (dedup check).
        if !scope.plugins.is_empty() {
            cfg.plugins.extend(scope.plugins);
            Self::ensure_unique_plugin_ids(&cfg.plugins)
                .map_err(AgentOsResolveError::from)
                .map_err(AgentOsRunError::from)?;
        }

        // 6. Overlay run-scoped tool registries (outside allowed_tools filtering).
        for reg in &scope.tool_registries {
            for (id, tool) in reg.snapshot() {
                tools.entry(id).or_insert(tool);
            }
        }

        cfg = cfg.with_step_tool_provider(Arc::new(StaticStepToolProvider::new(tools)));

        let run_ctx = RunContext::from_thread(&thread, run_config)
            .map_err(|e| AgentOsRunError::Loop(AgentLoopError::StateError(e.to_string())))?;

        Ok(PreparedRun {
            thread_id,
            run_id,
            config: cfg,
            run_ctx,
            cancellation_token: None,
            state_committer: Some(Arc::new(
                AgentStateStoreStateCommitter::new(agent_state_store.clone()),
            )),
        })
    }

    /// Execute a previously prepared run.
    pub fn execute_prepared(prepared: PreparedRun) -> Result<RunStream, AgentOsRunError> {
        let events = run_loop_stream(
            prepared.config,
            prepared.run_ctx,
            prepared.cancellation_token,
            prepared.state_committer,
        );
        Ok(RunStream {
            thread_id: prepared.thread_id,
            run_id: prepared.run_id,
            events,
        })
    }

    /// Run an agent from a [`RunRequest`].
    pub async fn run_stream(&self, request: RunRequest) -> Result<RunStream, AgentOsRunError> {
        let prepared = self.prepare_run(request, RunScope::default()).await?;
        Ok(Self::execute_prepared(prepared)?)
    }

    /// Run an agent from a [`RunRequest`] with run-scoped extensions.
    pub async fn run_stream_with_scope(
        &self,
        request: RunRequest,
        scope: RunScope,
    ) -> Result<RunStream, AgentOsRunError> {
        let prepared = self.prepare_run(request, scope).await?;
        Ok(Self::execute_prepared(prepared)?)
    }

    /// Deduplicate incoming messages against existing thread messages.
    ///
    /// Skips messages whose ID or tool_call_id already exists in the thread.
    fn dedup_messages(thread: &Thread, incoming: Vec<Message>) -> Vec<Message> {
        use std::collections::HashSet;

        let existing_ids: HashSet<&str> = thread
            .messages
            .iter()
            .filter_map(|m| m.id.as_deref())
            .collect();
        let existing_tool_call_ids: HashSet<&str> = thread
            .messages
            .iter()
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();

        incoming
            .into_iter()
            .filter(|m| {
                // Dedup tool messages by tool_call_id
                if let Some(ref tc_id) = m.tool_call_id {
                    if existing_tool_call_ids.contains(tc_id.as_str()) {
                        return false;
                    }
                }
                // Dedup by message id
                if let Some(ref id) = m.id {
                    if existing_ids.contains(id.as_str()) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    // --- Internal low-level helper (used by agent tools) ---

    pub(crate) fn run_stream_with_context(
        &self,
        agent_id: &str,
        thread: Thread,
        cancellation_token: Option<RunCancellationToken>,
        state_committer: Option<Arc<dyn StateCommitter>>,
    ) -> Result<impl futures::Stream<Item = AgentEvent> + Send, AgentOsRunError> {
        let (cfg, tools, thread, run_config) = self.resolve(agent_id, thread)?;
        let cfg = cfg.with_step_tool_provider(Arc::new(StaticStepToolProvider::new(tools)));
        let run_ctx = RunContext::from_thread(&thread, run_config)
            .map_err(|e| AgentOsRunError::Loop(AgentLoopError::StateError(e.to_string())))?;
        Ok(run_loop_stream(cfg, run_ctx, cancellation_token, state_committer))
    }
}
