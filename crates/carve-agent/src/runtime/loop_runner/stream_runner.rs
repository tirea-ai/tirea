use super::state_commit::PendingDeltaCommitContext;
use super::stream_core::{
    natural_result_payload, preallocate_tool_result_message_ids, resolve_stream_run_identity,
};
use super::*;

// Stream adapter layer:
// - drives provider I/O and plugin phases
// - emits AgentEvent stream
// - delegates deterministic state-machine helpers to `stream_core`

async fn drain_run_start_outbox_and_replay(
    mut thread: Thread,
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
    tool_descriptors: &[crate::contracts::traits::tool::ToolDescriptor],
) -> Result<(Thread, Vec<AgentEvent>), String> {
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

fn drain_loop_tick_outbox(thread: Thread) -> Result<(Thread, Vec<AgentEvent>), String> {
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

pub(super) fn run_loop_stream_impl_with_provider(
    provider: Arc<dyn ChatStreamProvider>,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    Box::pin(stream! {
    let mut thread = thread;
    let mut run_state = RunState::new();
    let stop_conditions = effective_stop_conditions(&config);
    let run_cancellation_token = run_ctx.run_cancellation_token().cloned();
    let state_committer = run_ctx.state_committer().cloned();
        let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(activity_tx));

    let tool_descriptors = tool_descriptors_for_config(&tools, &config);

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
                    yield AgentEvent::Error {
                        message: e.to_string(),
                    };
                    return;
                }
            };
        }

        macro_rules! terminate_stream_error {
            ($message:expr) => {{
                let (finalized, error, finish) = prepare_stream_error_termination(
                    thread,
                    &tool_descriptors,
                    &config.plugins,
                    &run_id,
                    $message,
                )
                .await;
                thread = finalized;
                emit_run_finished_delta!();
                yield error;
                yield finish;
                return;
            }};
        }

        macro_rules! finish_run {
            ($termination_expr:expr, $result_expr:expr) => {{
                let final_result = $result_expr;
                let final_termination = $termination_expr;
                thread = finalize_run_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: final_result,
                    termination: final_termination,
                };
                return;
            }};
        }

        // Phase: RunStart (use scoped block to manage borrow)
        match emit_phase_block(
            Phase::RunStart,
            &thread,
            &tool_descriptors,
            &config.plugins,
            |_| {},
        )
        .await
        {
            Ok(pending) => {
                thread = apply_pending_patches(thread, pending);
            }
            Err(e) => {
                terminate_stream_error!(e.to_string());
            }
        }

        yield AgentEvent::RunStart {
            thread_id: thread.id.clone(),
            run_id: run_id.clone(),
            parent_run_id: parent_run_id.clone(),
        };

        // Resume pending tool execution requested by plugins at run start.
        let (next_thread, run_start_events) = match drain_run_start_outbox_and_replay(
            thread.clone(),
            &tools,
            &config,
            &tool_descriptors,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                terminate_stream_error!(e);
            }
        };
        thread = next_thread;
        for event in run_start_events {
            yield event;
        }

        loop {
            let (next_thread, loop_tick_events) = match drain_loop_tick_outbox(thread.clone()) {
                Ok(v) => v,
                Err(e) => {
                    terminate_stream_error!(e);
                }
            };
            thread = next_thread;
            for event in loop_tick_events {
                yield event;
            }

            // Check cancellation at the top of each iteration.
            if is_run_cancelled(run_cancellation_token.as_ref()) {
                finish_run!(TerminationReason::Cancelled, None);
            }

            let prepared = match prepare_step_execution(&thread, &tool_descriptors, &config).await {
                Ok(v) => v,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
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
                        yield event;
                    }
                }
                finish_run!(
                    if pending_interaction.is_some() {
                        TerminationReason::PendingInteraction
                    } else {
                        TerminationReason::PluginRequested
                    },
                    None
                );
            }

            // Step boundary: starting LLM call
            let assistant_msg_id = gen_message_id();
            yield AgentEvent::StepStart { message_id: assistant_msg_id.clone() };

            // Stream LLM response with unified retry + fallback model strategy.
            let inference_span = prepared.tracing_span.unwrap_or_else(tracing::Span::none);
            let chat_options = config.chat_options.clone();
            let attempt_outcome = run_llm_with_retry_and_fallback(
                &config,
                run_cancellation_token.as_ref(),
                config.llm_retry_policy.retry_stream_start,
                "unknown llm stream start error",
                |model| {
                    let request =
                        build_request_for_filtered_tools(&messages, &tools, &filtered_tools);
                    let provider = provider.clone();
                    let chat_options = chat_options.clone();
                    async move {
                        provider
                            .exec_chat_stream_events(&model, request, chat_options.as_ref())
                            .await
                    }
                    .instrument(inference_span.clone())
                },
            )
            .await;

            let (chat_stream_events, inference_model) = match attempt_outcome {
                LlmAttemptOutcome::Success { value, model } => (value, model),
                LlmAttemptOutcome::Cancelled => {
                    finish_run!(TerminationReason::Cancelled, None);
                }
                LlmAttemptOutcome::Exhausted { last_error } => {
                    // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                    match apply_llm_error_cleanup(
                        &mut thread,
                        &tool_descriptors,
                        &config.plugins,
                        "llm_stream_start_error",
                        last_error.clone(),
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(phase_error) => {
                            terminate_stream_error!(phase_error.to_string());
                        }
                    }
                    terminate_stream_error!(last_error);
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
                                    yield AgentEvent::TextDelta { delta };
                                }
                                crate::runtime::streaming::StreamOutput::ToolCallStart { id, name } => {
                                    yield AgentEvent::ToolCallStart { id, name };
                                }
                                crate::runtime::streaming::StreamOutput::ToolCallDelta { id, args_delta } => {
                                    yield AgentEvent::ToolCallDelta { id, args_delta };
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                        match apply_llm_error_cleanup(
                            &mut thread,
                            &tool_descriptors,
                            &config.plugins,
                            "llm_stream_event_error",
                            e.to_string(),
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(phase_error) => {
                                terminate_stream_error!(phase_error.to_string());
                            }
                        }
                        terminate_stream_error!(e.to_string());
                    }
                }
            }

            let result = collector.finish();
            run_state.update_from_response(&result);
            let inference_duration_ms = inference_start.elapsed().as_millis() as u64;

            yield AgentEvent::InferenceComplete {
                model: inference_model,
                usage: result.usage.clone(),
                duration_ms: inference_duration_ms,
            };

            let step_meta = step_metadata(Some(run_id.clone()), run_state.completed_steps as u32);
            if let Err(e) = complete_step_after_inference(
                &mut thread,
                &result,
                step_meta.clone(),
                Some(assistant_msg_id.clone()),
                &tool_descriptors,
                &config.plugins,
            )
            .await
            {
                terminate_stream_error!(e.to_string());
            }

            if let Err(e) = pending_delta_commit
                .commit(&mut thread, CheckpointReason::AssistantTurnCommitted, false)
                .await
            {
                terminate_stream_error!(e.to_string());
            }

            // Step boundary: finished LLM call
            yield AgentEvent::StepEnd;

            mark_step_completed(&mut run_state);

            // Check if we need to execute tools
            if !result.needs_tools() {
                if is_run_cancelled(run_cancellation_token.as_ref()) {
                    finish_run!(TerminationReason::Cancelled, None);
                }
                if let Some(reason) =
                    stop_reason_for_step(&run_state, &result, &thread, &stop_conditions)
                {
                    finish_run!(TerminationReason::Stopped(reason), None);
                }
                let result_value = natural_result_payload(&result.text);
                finish_run!(TerminationReason::NaturalEnd, result_value);
            }

            // Emit ToolCallReady for each finalized tool call
            for tc in &result.tool_calls {
                yield AgentEvent::ToolCallReady {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                };
            }

            // Execute tools with phase hooks
            let tool_context = match prepare_tool_execution_context(&thread, Some(&config)) {
                Ok(ctx) => ctx,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };
            let sid_for_tools = thread.id.clone();
            let mut tool_future: Pin<Box<dyn Future<Output = Result<Vec<ToolExecutionResult>, AgentLoopError>> + Send>> =
                Box::pin(execute_tool_calls_with_phases(
                    &tools,
                    &result.tool_calls,
                    &tool_context.state,
                    &tool_descriptors,
                    &config.plugins,
                    config.parallel_tools,
                    Some(activity_manager.clone()),
                    Some(&tool_context.scope),
                    &sid_for_tools,
                ));
            let mut activity_closed = false;
            let results = loop {
                tokio::select! {
                    _ = async {
                        if let Some(ref token) = run_cancellation_token {
                            token.cancelled().await;
                        } else {
                            futures::future::pending::<()>().await;
                        }
                    } => {
                        finish_run!(TerminationReason::Cancelled, None);
                    }
                    activity = activity_rx.recv(), if !activity_closed => {
                        match activity {
                            Some(event) => {
                                yield event;
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
                yield event;
            }

            let results = match results {
                Ok(r) => r,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };

            // Emit pending interaction event(s) first.
            for exec_result in &results {
                if let Some(ref interaction) = exec_result.pending_interaction {
                    yield AgentEvent::InteractionRequested {
                        interaction: interaction.clone(),
                    };
                    yield AgentEvent::Pending {
                        interaction: interaction.clone(),
                    };
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
                config.parallel_tools,
                Some(&tool_msg_ids),
            ) {
                Ok(a) => a,
                Err(e) => {
                    thread = thread_before_apply;
                    terminate_stream_error!(e.to_string());
                }
            };
            thread = applied.thread;

            if let Err(e) = pending_delta_commit
                .commit(&mut thread, CheckpointReason::ToolResultsCommitted, false)
                .await
            {
                terminate_stream_error!(e.to_string());
            }

            // Emit non-pending tool results (pending ones pause the run).
            for exec_result in &results {
                if exec_result.pending_interaction.is_none() {
                    yield AgentEvent::ToolCallDone {
                        id: exec_result.execution.call.id.clone(),
                        result: exec_result.execution.result.clone(),
                        patch: exec_result.execution.patch.clone(),
                        message_id: tool_msg_ids.get(&exec_result.execution.call.id).cloned().unwrap_or_default(),
                    };
                }
            }

            // Emit state snapshot when we mutated state (tool patches or AgentState pending/clear).
            if let Some(snapshot) = applied.state_snapshot {
                yield AgentEvent::StateSnapshot { snapshot };
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
