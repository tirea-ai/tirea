use super::*;

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
    /// 1. AgentState loading/creation from storage
    /// 2. Message deduplication and appending
    /// 3. Persisting pre-run state
    /// 4. Agent resolution and run-context creation
    pub async fn prepare_run(&self, request: RunRequest) -> Result<PreparedRun, AgentOsRunError> {
        self.prepare_run_with_extensions(request, RunExtensions::default())
            .await
    }

    /// Prepare a request for execution with run-scoped extensions.
    pub async fn prepare_run_with_extensions(
        &self,
        mut request: RunRequest,
        extensions: RunExtensions,
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
                    AgentState::with_initial_state(thread_id.clone(), state)
                } else {
                    AgentState::new(thread_id.clone())
                };
                let committed = agent_state_store.create(&thread).await?;
                (thread, committed.version)
            }
        };

        // 2. Set resource_id on thread if provided
        if let Some(ref resource_id) = request.resource_id {
            thread.resource_id = Some(resource_id.clone());
        }

        // 3. Set run identity on thread runtime
        let _ = thread.scope.set("run_id", run_id.clone());
        if let Some(parent) = parent_run_id.as_deref() {
            let _ = thread.scope.set("parent_run_id", parent.to_string());
        }

        // 4. Deduplicate and append inbound messages
        let deduped = Self::dedup_messages(&thread, request.messages);
        if !deduped.is_empty() {
            thread = thread.with_messages(deduped);
        }

        // 5. Persist pending changes (user messages + frontend state snapshot)
        let pending = thread.take_pending();
        if !pending.is_empty() || state_snapshot_for_delta.is_some() {
            let changeset = crate::contracts::AgentChangeSet::from_parts(
                Some(version),
                run_id.clone(),
                parent_run_id.clone(),
                CheckpointReason::UserMessage,
                pending.messages,
                pending.patches,
                state_snapshot_for_delta,
            );
            let committed = agent_state_store.append(&thread_id, &changeset).await?;
            version = committed.version;
        }
        let version_timestamp = thread.metadata.version_timestamp;
        crate::runtime::loop_runner::set_thread_state_version(
            &mut thread,
            version,
            version_timestamp,
        );

        // 6. Resolve static wiring, then merge run-scoped extensions.
        let (client, cfg, tools, thread) = self.resolve(&request.agent_id, thread)?;
        let (cfg, tools) = self
            .apply_run_extensions(cfg, tools, extensions)
            .map_err(AgentOsResolveError::from)
            .map_err(AgentOsRunError::from)?;
        let run_ctx = RunContext::default().with_state_committer(Arc::new(
            AgentStateStoreStateCommitter::new(agent_state_store.clone()),
        ));

        Ok(PreparedRun {
            thread_id,
            run_id,
            client,
            config: cfg,
            tools,
            thread,
            run_ctx,
        })
    }

    /// Execute a previously prepared run.
    pub fn execute_prepared(prepared: PreparedRun) -> RunStream {
        let events = run_loop_stream(
            prepared.client,
            prepared.config,
            prepared.thread,
            prepared.tools,
            prepared.run_ctx,
        );
        RunStream {
            thread_id: prepared.thread_id,
            run_id: prepared.run_id,
            events,
        }
    }

    /// Run an agent from a [`RunRequest`].
    pub async fn run_stream(&self, request: RunRequest) -> Result<RunStream, AgentOsRunError> {
        self.run_stream_with_extensions(request, RunExtensions::default())
            .await
    }

    /// Run an agent from a [`RunRequest`] with run-scoped extensions.
    pub async fn run_stream_with_extensions(
        &self,
        request: RunRequest,
        extensions: RunExtensions,
    ) -> Result<RunStream, AgentOsRunError> {
        let prepared = self
            .prepare_run_with_extensions(request, extensions)
            .await?;
        Ok(Self::execute_prepared(prepared))
    }

    /// Deduplicate incoming messages against existing thread messages.
    ///
    /// Skips messages whose ID or tool_call_id already exists in the thread.
    fn dedup_messages(thread: &AgentState, incoming: Vec<Message>) -> Vec<Message> {
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
        thread: AgentState,
        run_ctx: RunContext,
    ) -> Result<impl futures::Stream<Item = AgentEvent> + Send, AgentOsResolveError> {
        let (client, cfg, tools, thread) = self.resolve(agent_id, thread)?;
        Ok(run_loop_stream(client, cfg, thread, tools, run_ctx))
    }
}
