use super::state_commit::PendingDeltaCommitContext;
use super::stream_core::{preallocate_tool_result_message_ids, resolve_stream_run_identity};
use super::*;

// Stream adapter layer:
// - drives provider I/O and plugin phases
// - emits AgentEvent stream
// - delegates deterministic state-machine helpers to `stream_core`

async fn drain_run_start_outbox_and_replay(
    mut thread: AgentState,
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
) -> Result<(AgentState, Vec<AgentEvent>), String> {
    let (next_thread, outbox) =
        drain_agent_outbox(thread.clone(), "agent_outbox_run_start").map_err(|e| e.to_string())?;
    thread = next_thread;

    let mut events = Vec::new();
    for resolution in outbox.interaction_resolutions {
        events.push(AgentEvent::InteractionResolved {
            interaction_id: resolution.interaction_id,
            result: resolution.result,
        });
    }

    let replay_calls = outbox.replay_tool_calls;
    if replay_calls.is_empty() {
        return Ok((thread, events));
    }

    let mut replay_state_changed = false;
    for tool_call in &replay_calls {
        let state = thread.rebuild_state().map_err(|e| {
            format!(
                "failed to rebuild state before replaying tool '{}': {e}",
                tool_call.id
            )
        })?;

        let tool = tools.get(&tool_call.name).cloned();
        let rt_for_replay = scope_with_tool_caller_context(&thread, &state, Some(config))
            .map_err(|e| e.to_string())?;
        let replay_result = execute_single_tool_with_phases(
            tool.as_deref(),
            tool_call,
            &state,
            tool_descriptors,
            &config.plugins,
            None,
            Some(&rt_for_replay),
            &thread.id,
            &thread.messages,
            agent_state_version(&thread),
            thread.run_overlay(),
        )
        .await
        .map_err(|e| e.to_string())?;

        if replay_result.pending_interaction.is_some() {
            return Err(format!(
                "replayed tool '{}' requested pending interaction",
                tool_call.id
            ));
        }

        // Append real replay tool result as a new tool message (append-only log).
        let replay_msg_id = gen_message_id();
        let mut replay_mutations = ThreadMutationBatch::default().with_message(
            tool_response(&tool_call.id, &replay_result.execution.result)
                .with_id(replay_msg_id.clone()),
        );

        // Preserve reminder emission semantics for replayed tool calls.
        if !replay_result.reminders.is_empty() {
            let msgs = replay_result.reminders.iter().map(|reminder| {
                Message::internal_system(format!("<system-reminder>{}</system-reminder>", reminder))
            });
            replay_mutations = replay_mutations.with_messages(msgs);
        }

        if let Some(patch) = replay_result.execution.patch.clone() {
            replay_state_changed = true;
            replay_mutations = replay_mutations.with_patch(patch);
        }
        if !replay_result.pending_patches.is_empty() {
            replay_state_changed = true;
            replay_mutations = replay_mutations.with_patches(replay_result.pending_patches.clone());
        }
        thread = reduce_thread_mutations(thread, replay_mutations);

        events.push(AgentEvent::ToolCallDone {
            id: tool_call.id.clone(),
            result: replay_result.execution.result,
            patch: replay_result.execution.patch,
            message_id: replay_msg_id,
        });
    }

    // Clear pending_interaction state after replaying tools.
    let state = thread
        .rebuild_state()
        .map_err(|e| format!("failed to rebuild state after replay: {e}"))?;
    let clear_patch = clear_agent_pending_interaction(&state);
    if !clear_patch.patch().is_empty() {
        replay_state_changed = true;
        thread = reduce_thread_mutations(
            thread,
            ThreadMutationBatch::default().with_patch(clear_patch),
        );
    }

    if replay_state_changed {
        let snapshot = thread
            .rebuild_state()
            .map_err(|e| format!("failed to rebuild replay snapshot: {e}"))?;
        events.push(AgentEvent::StateSnapshot { snapshot });
    }

    Ok((thread, events))
}

fn drain_loop_tick_outbox(thread: AgentState) -> Result<(AgentState, Vec<AgentEvent>), String> {
    let (next_thread, outbox) =
        drain_agent_outbox(thread.clone(), "agent_outbox_loop_tick").map_err(|e| e.to_string())?;

    if !outbox.replay_tool_calls.is_empty() {
        return Err("unexpected replay_tool_calls outside run start".to_string());
    }

    let events = outbox
        .interaction_resolutions
        .into_iter()
        .map(|resolution| AgentEvent::InteractionResolved {
            interaction_id: resolution.interaction_id,
            result: resolution.result,
        })
        .collect::<Vec<_>>();
    Ok((next_thread, events))
}

#[derive(Debug)]
struct StreamEventEmitter {
    run_id: String,
    thread_id: String,
    parent_run_id: Option<String>,
    seq: u64,
    step_index: u32,
    current_step_id: Option<String>,
}

impl StreamEventEmitter {
    fn new(run_id: String, thread_id: String, parent_run_id: Option<String>) -> Self {
        Self {
            run_id,
            thread_id,
            parent_run_id,
            seq: 0,
            step_index: 0,
            current_step_id: None,
        }
    }

    fn run_start(&mut self) -> AgentEvent {
        self.emit(AgentEvent::RunStart {
            thread_id: self.thread_id.clone(),
            run_id: self.run_id.clone(),
            parent_run_id: self.parent_run_id.clone(),
        })
    }

    fn run_finish(&mut self, outcome: LoopOutcome) -> AgentEvent {
        self.emit(outcome.to_run_finish_event(self.run_id.clone()))
    }

    fn step_start(&mut self, message_id: String) -> AgentEvent {
        self.current_step_id = Some(format!("step:{}", self.step_index));
        self.emit(AgentEvent::StepStart { message_id })
    }

    fn step_end(&mut self) -> AgentEvent {
        let event = self.emit(AgentEvent::StepEnd);
        self.step_index = self.step_index.saturating_add(1);
        self.current_step_id = None;
        event
    }

    fn emit_existing(&mut self, event: AgentEvent) -> AgentEvent {
        self.emit(event)
    }

    fn emit(&mut self, event: AgentEvent) -> AgentEvent {
        let seq = self.seq;
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        crate::contracts::runtime::event::register_runtime_event_envelope_meta(
            &event,
            &self.run_id,
            &self.thread_id,
            seq,
            timestamp_ms,
            self.current_step_id.clone(),
        );
        self.seq = self.seq.saturating_add(1);
        tracing::trace!(
            run_id = %self.run_id,
            thread_id = %self.thread_id,
            parent_run_id = %self.parent_run_id.clone().unwrap_or_default(),
            seq,
            timestamp_ms,
            step_id = %self.current_step_id.clone().unwrap_or_default(),
            event_type = %event_type_name(&event),
            "emit agent event"
        );
        event
    }
}

fn event_type_name(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::RunStart { .. } => "run_start",
        AgentEvent::RunFinish { .. } => "run_finish",
        AgentEvent::TextDelta { .. } => "text_delta",
        AgentEvent::ToolCallStart { .. } => "tool_call_start",
        AgentEvent::ToolCallDelta { .. } => "tool_call_delta",
        AgentEvent::ToolCallReady { .. } => "tool_call_ready",
        AgentEvent::ToolCallDone { .. } => "tool_call_done",
        AgentEvent::StepStart { .. } => "step_start",
        AgentEvent::StepEnd => "step_end",
        AgentEvent::InferenceComplete { .. } => "inference_complete",
        AgentEvent::StateSnapshot { .. } => "state_snapshot",
        AgentEvent::StateDelta { .. } => "state_delta",
        AgentEvent::MessagesSnapshot { .. } => "messages_snapshot",
        AgentEvent::ActivitySnapshot { .. } => "activity_snapshot",
        AgentEvent::ActivityDelta { .. } => "activity_delta",
        AgentEvent::InteractionRequested { .. } => "interaction_requested",
        AgentEvent::InteractionResolved { .. } => "interaction_resolved",
        AgentEvent::Pending { .. } => "pending",
        AgentEvent::Error { .. } => "error",
    }
}

pub(super) fn run_loop_stream_impl_with_provider(
    provider: Arc<dyn ChatStreamProvider>,
    config: AgentConfig,
    thread: AgentState,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    Box::pin(stream! {
    let mut thread = thread;
    let mut run_state = RunState::new();
    let mut last_text = String::new();
    let stop_conditions = effective_stop_conditions(&config);
    let run_cancellation_token = run_ctx.run_cancellation_token().cloned();
    let state_committer = run_ctx.state_committer().cloned();
    let step_tool_provider = step_tool_provider_for_run(&config, &tools);
        let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(activity_tx));

        // Resolve run_id: from runtime if pre-set, otherwise generate.
        // NOTE: runtime is mutated in-place here. This is intentional —
        // runtime is transient (not persisted) and the owned-builder pattern
        // (`with_runtime`) is impractical inside the loop where `thread` is
        // borrowed across yield points.
        let run_identity = resolve_stream_run_identity(&mut thread);
        let run_id = run_identity.run_id;
        let parent_run_id = run_identity.parent_run_id;
        let pending_delta_commit = PendingDeltaCommitContext::new(
            &run_id,
            parent_run_id.as_deref(),
            state_committer.as_ref(),
        );
        let mut emitter =
            StreamEventEmitter::new(run_id.clone(), thread.id.clone(), parent_run_id.clone());
        let mut active_tool_snapshot = match resolve_step_tool_snapshot(&step_tool_provider, &thread).await {
            Ok(snapshot) => snapshot,
            Err(e) => {
                let message = e.to_string();
                yield emitter.emit_existing(AgentEvent::Error {
                    message: message.clone(),
                });
                let outcome = build_loop_outcome(
                    thread,
                    TerminationReason::Error,
                    None,
                    &run_state,
                    Some(outcome::LoopFailure::State(message)),
                );
                yield emitter.run_finish(outcome);
                return;
            }
        };
        let mut active_tool_descriptors = active_tool_snapshot.descriptors.clone();

        macro_rules! emit_run_finished_delta {
            () => {
                if let Err(e) = pending_delta_commit
                    .commit(&mut thread, CheckpointReason::RunFinished, true)
                    .await
                {
                    tracing::warn!(error = %e, "failed to commit run-finished delta");
                }
            };
        }

        macro_rules! ensure_run_finished_delta_or_error {
            () => {
                if let Err(e) = pending_delta_commit
                    .commit(&mut thread, CheckpointReason::RunFinished, true)
                    .await
                {
                    yield emitter.emit_existing(AgentEvent::Error {
                        message: e.to_string(),
                    });
                    return;
                }
            };
        }

        macro_rules! terminate_stream_error {
            ($failure:expr, $message:expr) => {{
                let failure = $failure;
                let message = $message;
                thread = finalize_run_end(thread, &active_tool_descriptors, &config.plugins).await;
                emit_run_finished_delta!();
                let outcome = build_loop_outcome(
                    thread,
                    TerminationReason::Error,
                    Some(last_text.clone()),
                    &run_state,
                    Some(failure),
                );
                yield emitter.emit_existing(AgentEvent::Error {
                    message: message.clone(),
                });
                yield emitter.run_finish(outcome);
                return;
            }};
        }

        macro_rules! finish_run {
            ($termination_expr:expr, $response_expr:expr) => {{
                let final_termination = $termination_expr;
                let final_response = $response_expr;
                thread = finalize_run_end(thread, &active_tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                let outcome = build_loop_outcome(
                    thread,
                    final_termination,
                    final_response,
                    &run_state,
                    None,
                );
                yield emitter.run_finish(outcome);
                return;
            }};
        }

        // Phase: RunStart (use scoped block to manage borrow)
        match emit_phase_block(
            Phase::RunStart,
            &thread,
            &active_tool_descriptors,
            &config.plugins,
            |_| {},
        )
            .await
        {
            Ok(pending) => {
                thread = apply_pending_patches(thread, pending);
            }
            Err(e) => {
                let message = e.to_string();
                terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
            }
        }

        yield emitter.run_start();

        // Resume pending tool execution requested by plugins at run start.
        let (next_thread, run_start_events) = match drain_run_start_outbox_and_replay(
            thread.clone(),
            &active_tool_snapshot.tools,
            &config,
            &active_tool_descriptors,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                let message = e;
                terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
            }
        };
        thread = next_thread;
        for event in run_start_events {
            yield emitter.emit_existing(event);
        }

        loop {
            let (next_thread, loop_tick_events) = match drain_loop_tick_outbox(thread.clone()) {
                Ok(v) => v,
                Err(e) => {
                    let message = e;
                    terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                }
            };
            thread = next_thread;
            for event in loop_tick_events {
                yield emitter.emit_existing(event);
            }

            // Check cancellation at the top of each iteration.
            if is_run_cancelled(run_cancellation_token.as_ref()) {
                finish_run!(TerminationReason::Cancelled, None);
            }

            active_tool_snapshot = match resolve_step_tool_snapshot(&step_tool_provider, &thread).await {
                Ok(snapshot) => snapshot,
                Err(e) => {
                    let message = e.to_string();
                    terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                }
            };
            active_tool_descriptors = active_tool_snapshot.descriptors.clone();

            let prepared = match prepare_step_execution(&thread, &active_tool_descriptors, &config).await {
                Ok(v) => v,
                Err(e) => {
                    let message = e.to_string();
                    terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                }
            };
            thread = apply_pending_patches(thread, prepared.pending_patches);
            let messages = prepared.messages;
            let filtered_tools = prepared.filtered_tools;

            // Skip inference if requested
            if prepared.skip_inference {
                let pending_interaction = pending_interaction_from_thread(&thread);
                if let Some(interaction) = pending_interaction.clone() {
                    for event in interaction_requested_pending_events(&interaction) {
                        yield emitter.emit_existing(event);
                    }
                }
                finish_run!(
                    if pending_interaction.is_some() {
                        TerminationReason::PendingInteraction
                    } else {
                        TerminationReason::PluginRequested
                    },
                    Some(last_text.clone())
                );
            }

            // Step boundary: starting LLM call
            let assistant_msg_id = gen_message_id();
            yield emitter.step_start(assistant_msg_id.clone());

            // Stream LLM response with unified retry + fallback model strategy.
            let chat_options = config.chat_options.clone();
            let attempt_outcome = run_llm_with_retry_and_fallback(
                &config,
                run_cancellation_token.as_ref(),
                config.llm_retry_policy.retry_stream_start,
                "unknown llm stream start error",
                |model| {
                    let request =
                        build_request_for_filtered_tools(&messages, &active_tool_snapshot.tools, &filtered_tools);
                    let provider = provider.clone();
                    let chat_options = chat_options.clone();
                    async move {
                        provider
                            .exec_chat_stream_events(&model, request, chat_options.as_ref())
                            .await
                    }
                },
            )
            .await;

            let (chat_stream_events, inference_model) = match attempt_outcome {
                LlmAttemptOutcome::Success {
                    value,
                    model,
                    attempts,
                } => {
                    run_state.record_llm_attempts(attempts);
                    (value, model)
                }
                LlmAttemptOutcome::Cancelled => {
                    finish_run!(TerminationReason::Cancelled, None);
                }
                LlmAttemptOutcome::Exhausted {
                    last_error,
                    attempts,
                } => {
                    run_state.record_llm_attempts(attempts);
                    // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                        match apply_llm_error_cleanup(
                            &mut thread,
                            &active_tool_descriptors,
                            &config.plugins,
                            "llm_stream_start_error",
                            last_error.clone(),
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(phase_error) => {
                            let message = phase_error.to_string();
                            terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                        }
                    }
                    let message = last_error;
                    terminate_stream_error!(outcome::LoopFailure::Llm(message.clone()), message);
                }
            };

            // Collect streaming response
            let inference_start = std::time::Instant::now();
            let mut collector = StreamCollector::new();
            let mut chat_stream = chat_stream_events;

            loop {
                let next_event = if let Some(ref token) = run_cancellation_token {
                    tokio::select! {
                        _ = token.cancelled() => {
                            finish_run!(TerminationReason::Cancelled, None);
                        }
                        ev = chat_stream.next() => ev,
                    }
                } else {
                    chat_stream.next().await
                };

                let Some(event_result) = next_event else {
                    break;
                };

                match event_result {
                    Ok(event) => {
                        if let Some(output) = collector.process(event) {
                            match output {
                                crate::runtime::streaming::StreamOutput::TextDelta(delta) => {
                                    yield emitter.emit_existing(AgentEvent::TextDelta { delta });
                                }
                                crate::runtime::streaming::StreamOutput::ToolCallStart { id, name } => {
                                    yield emitter.emit_existing(AgentEvent::ToolCallStart { id, name });
                                }
                                crate::runtime::streaming::StreamOutput::ToolCallDelta { id, args_delta } => {
                                    yield emitter.emit_existing(AgentEvent::ToolCallDelta { id, args_delta });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                        match apply_llm_error_cleanup(
                            &mut thread,
                            &active_tool_descriptors,
                            &config.plugins,
                            "llm_stream_event_error",
                            e.to_string(),
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(phase_error) => {
                                let message = phase_error.to_string();
                                terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                            }
                        }
                        let message = e.to_string();
                        terminate_stream_error!(outcome::LoopFailure::Llm(message.clone()), message);
                    }
                }
            }

            let result = collector.finish();
            last_text = result.text.clone();
            run_state.update_from_response(&result);
            let inference_duration_ms = inference_start.elapsed().as_millis() as u64;

            yield emitter.emit_existing(AgentEvent::InferenceComplete {
                model: inference_model,
                usage: result.usage.clone(),
                duration_ms: inference_duration_ms,
            });

            let step_meta = step_metadata(Some(run_id.clone()), run_state.completed_steps as u32);
            if let Err(e) = complete_step_after_inference(
                &mut thread,
                &result,
                step_meta.clone(),
                Some(assistant_msg_id.clone()),
                &active_tool_descriptors,
                &config.plugins,
            )
            .await
            {
                let message = e.to_string();
                terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
            }

            if let Err(e) = pending_delta_commit
                .commit(&mut thread, CheckpointReason::AssistantTurnCommitted, false)
                .await
            {
                let message = e.to_string();
                terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
            }

            // Step boundary: finished LLM call
            yield emitter.step_end();

            mark_step_completed(&mut run_state);

            // Check if we need to execute tools
            if !result.needs_tools() {
                run_state.record_step_without_tools();
                if is_run_cancelled(run_cancellation_token.as_ref()) {
                    finish_run!(TerminationReason::Cancelled, None);
                }
                if let Some(reason) =
                    stop_reason_for_step(&run_state, &result, &thread, &stop_conditions)
                {
                    finish_run!(TerminationReason::Stopped(reason), None);
                }
                finish_run!(TerminationReason::NaturalEnd, Some(last_text.clone()));
            }

            // Emit ToolCallReady for each finalized tool call
            for tc in &result.tool_calls {
                yield emitter.emit_existing(AgentEvent::ToolCallReady {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });
            }

            // Execute tools with phase hooks
            let tool_context = match prepare_tool_execution_context(&thread, Some(&config)) {
                Ok(ctx) => ctx,
                Err(e) => {
                    let message = e.to_string();
                    terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                }
            };
            let sid_for_tools = thread.id.clone();
            let thread_messages_for_tools = thread.messages.clone();
            let thread_version_for_tools = agent_state_version(&thread);
            let mut tool_future: Pin<
                Box<dyn Future<Output = Result<Vec<ToolExecutionResult>, AgentLoopError>> + Send>,
            > = Box::pin(async {
                config
                    .tool_executor
                    .execute(ToolExecutionRequest {
                        tools: &active_tool_snapshot.tools,
                        calls: &result.tool_calls,
                        state: &tool_context.state,
                        tool_descriptors: &active_tool_descriptors,
                        plugins: &config.plugins,
                        activity_manager: Some(activity_manager.clone()),
                        scope: Some(&tool_context.scope),
                        thread_id: &sid_for_tools,
                        thread_messages: &thread_messages_for_tools,
                        state_version: thread_version_for_tools,
                        cancellation_token: run_cancellation_token.as_ref(),
                        run_overlay: tool_context.run_overlay.clone(),
                    })
                    .await
                    .map_err(AgentLoopError::from)
            });
            let mut activity_closed = false;
            let results = loop {
                tokio::select! {
                    activity = activity_rx.recv(), if !activity_closed => {
                        match activity {
                            Some(event) => {
                                yield emitter.emit_existing(event);
                            }
                            None => {
                                activity_closed = true;
                            }
                        }
                    }
                    res = &mut tool_future => {
                        break res;
                    }
                }
            };

            while let Ok(event) = activity_rx.try_recv() {
                yield emitter.emit_existing(event);
            }

            let results = match results {
                Ok(r) => r,
                Err(AgentLoopError::Cancelled { .. }) => {
                    finish_run!(TerminationReason::Cancelled, None);
                }
                Err(e) => {
                    let message = e.to_string();
                    terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                }
            };

            // Emit pending interaction event(s) first.
            for exec_result in &results {
                if let Some(ref interaction) = exec_result.pending_interaction {
                    for event in interaction_requested_pending_events(interaction) {
                        yield emitter.emit_existing(event);
                    }
                }
            }
            let thread_before_apply = thread.clone();
            // Pre-generate message IDs for tool results so streaming events
            // and stored Messages share the same ID.
            let tool_msg_ids = preallocate_tool_result_message_ids(&results);

            let applied = match apply_tool_results_impl(
                thread,
                &results,
                Some(step_meta),
                config.tool_executor.requires_parallel_patch_conflict_check(),
                Some(&tool_msg_ids),
            ) {
                Ok(a) => a,
                Err(e) => {
                    thread = thread_before_apply;
                    let message = e.to_string();
                    terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
                }
            };
            thread = applied.thread;

            if let Err(e) = pending_delta_commit
                .commit(&mut thread, CheckpointReason::ToolResultsCommitted, false)
                .await
            {
                let message = e.to_string();
                terminate_stream_error!(outcome::LoopFailure::State(message.clone()), message);
            }

            // Emit non-pending tool results (pending ones pause the run).
            for exec_result in &results {
                if exec_result.pending_interaction.is_none() {
                    yield emitter.emit_existing(AgentEvent::ToolCallDone {
                        id: exec_result.execution.call.id.clone(),
                        result: exec_result.execution.result.clone(),
                        patch: exec_result.execution.patch.clone(),
                        message_id: tool_msg_ids.get(&exec_result.execution.call.id).cloned().unwrap_or_default(),
                    });
                }
            }

            // Emit state snapshot when we mutated state (tool patches or AgentState pending/clear).
            if let Some(snapshot) = applied.state_snapshot {
                yield emitter.emit_existing(AgentEvent::StateSnapshot { snapshot });
            }

            // If there are pending interactions, pause the loop.
            // Client must respond and start a new run to continue.
            if applied.pending_interaction.is_some() {
                finish_run!(TerminationReason::PendingInteraction, None);
            }

            // Track tool step metrics for stop condition evaluation.
            let error_count = results
                .iter()
                .filter(|r| r.execution.result.is_error())
                .count();
            run_state.record_tool_step(&result.tool_calls, error_count);

            // Check stop conditions.
            if let Some(reason) = stop_reason_for_step(&run_state, &result, &thread, &stop_conditions)
            {
                finish_run!(TerminationReason::Stopped(reason), None);
            }
        }
    })
}
