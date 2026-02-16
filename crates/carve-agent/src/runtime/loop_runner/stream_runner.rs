use super::stream_core::{
    natural_result_payload, pending_interaction_from_thread, preallocate_tool_result_message_ids,
    resolve_stream_run_identity,
};
use super::*;

// Stream adapter layer:
// - drives provider I/O and plugin phases
// - emits AgentEvent stream
// - delegates deterministic state-machine helpers to `stream_core`

pub(super) fn run_loop_stream_impl_with_provider(
    provider: Arc<dyn ChatStreamProvider>,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    Box::pin(stream! {
    let mut thread = thread;
    let mut loop_state = LoopState::new();
    let stop_conditions = effective_stop_conditions(&config);
    let run_cancellation_token = run_ctx.run_cancellation_token().cloned();
    let state_committer = run_ctx.state_committer().cloned();
        let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(activity_tx));

    let tool_descriptors = tool_descriptors_for_config(&tools, &config);

        // Resolve run_id: from runtime if pre-set, otherwise generate.
        // NOTE: runtime is mutated in-place here. This is intentional —
        // runtime is transient (not persisted) and the owned-builder pattern
        // (`with_runtime`) is impractical inside the loop where `session` is
        // borrowed across yield points.
        let run_identity = resolve_stream_run_identity(&mut thread);
        let run_id = run_identity.run_id;
        let parent_run_id = run_identity.parent_run_id;

        macro_rules! emit_run_finished_delta {
            () => {
                if let Err(e) = commit_pending_delta(
                    &mut thread,
                    CheckpointReason::RunFinished,
                    true,
                    &run_id,
                    parent_run_id.as_deref(),
                    state_committer.as_ref(),
                )
                .await
                {
                    tracing::warn!(error = %e, "failed to commit run-finished delta");
                }
            };
        }

        macro_rules! ensure_run_finished_delta_or_error {
            () => {
                if let Err(e) = commit_pending_delta(
                    &mut thread,
                    CheckpointReason::RunFinished,
                    true,
                    &run_id,
                    parent_run_id.as_deref(),
                    state_committer.as_ref(),
                )
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

        // Phase: SessionStart (use scoped block to manage borrow)
        match emit_phase_block(
            Phase::SessionStart,
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

        let (next_thread, outbox) =
            match drain_agent_outbox(thread.clone(), "agent_outbox_session_start")
        {
            Ok(v) => v,
            Err(e) => {
                terminate_stream_error!(e.to_string());
            }
        };
        thread = next_thread;
        for resolution in outbox.interaction_resolutions {
            yield AgentEvent::InteractionResolved {
                interaction_id: resolution.interaction_id,
                result: resolution.result,
            };
        }

        // Resume pending tool execution via plugin mechanism.
        // Plugins request replay by writing AgentState.replay_tool_calls.
        // The loop executes those calls at session start.
        let replay_calls = outbox.replay_tool_calls;
        if !replay_calls.is_empty() {
            let mut replay_state_changed = false;
            for tool_call in &replay_calls {
                let state = match thread.rebuild_state() {
                    Ok(s) => s,
                    Err(e) => {
                        terminate_stream_error!(format!(
                            "failed to rebuild state before replaying tool '{}': {e}",
                            tool_call.id
                        ));
                    }
                };

                let tool = tools.get(&tool_call.name).cloned();
                let rt_for_replay =
                    match runtime_with_tool_caller_context(&thread, &state, Some(&config)) {
                    Ok(rt) => rt,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };
                let replay_result = match execute_single_tool_with_phases(
                    tool.as_deref(),
                    tool_call,
                    &state,
                    &tool_descriptors,
                    &config.plugins,
                    None,
                    Some(&rt_for_replay),
                    &thread.id,
                )
                .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };

                if replay_result.pending_interaction.is_some() {
                    terminate_stream_error!(format!(
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
                        Message::internal_system(format!(
                            "<system-reminder>{}</system-reminder>",
                            reminder
                        ))
                    });
                    replay_mutations = replay_mutations.with_messages(msgs);
                }

                if let Some(patch) = replay_result.execution.patch.clone() {
                    replay_state_changed = true;
                    replay_mutations = replay_mutations.with_patch(patch);
                }
                if !replay_result.pending_patches.is_empty() {
                    replay_state_changed = true;
                    replay_mutations =
                        replay_mutations.with_patches(replay_result.pending_patches.clone());
                }
                thread = reduce_thread_mutations(thread, replay_mutations);

                yield AgentEvent::ToolCallDone {
                    id: tool_call.id.clone(),
                    result: replay_result.execution.result,
                    patch: replay_result.execution.patch,
                    message_id: replay_msg_id,
                };
            }

            // Clear pending_interaction state after replaying tools.
            let state = match thread.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    terminate_stream_error!(format!("failed to rebuild state after replay: {e}"));
                }
            };

            let clear_patch = clear_agent_pending_interaction(&state);
            if !clear_patch.patch().is_empty() {
                replay_state_changed = true;
                thread = reduce_thread_mutations(
                    thread,
                    ThreadMutationBatch::default().with_patch(clear_patch),
                );
            }

            if replay_state_changed {
                let snapshot = match thread.rebuild_state() {
                    Ok(s) => s,
                    Err(e) => {
                        terminate_stream_error!(format!("failed to rebuild replay snapshot: {e}"));
                    }
                };
                yield AgentEvent::StateSnapshot {
                    snapshot,
                };
            }
        }

        loop {
            let (next_thread, outbox) =
                match drain_agent_outbox(thread.clone(), "agent_outbox_loop_tick")
            {
                Ok(v) => v,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };
            thread = next_thread;
            for resolution in outbox.interaction_resolutions {
                yield AgentEvent::InteractionResolved {
                    interaction_id: resolution.interaction_id,
                    result: resolution.result,
                };
            }
            if !outbox.replay_tool_calls.is_empty() {
                terminate_stream_error!(
                    "unexpected replay_tool_calls outside session start".to_string()
                );
            }

            // Check cancellation at the top of each iteration.
            if let Some(ref token) = run_cancellation_token {
                if token.is_cancelled() {
                    thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                    ensure_run_finished_delta_or_error!();
                    yield AgentEvent::RunFinish {
                        thread_id: thread.id.clone(),
                        run_id: run_id.clone(),
                        result: None,
                        termination: TerminationReason::Cancelled,
                    };
                    return;
                }
            }

            // Phase: StepStart and BeforeInference (collect messages and tools filter)
            let (messages, filtered_tools, skip_inference, tracing_span, pending) =
                match run_step_prepare_phases(&thread, &tool_descriptors, &config).await {
                    Ok(v) => v,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };
            thread = apply_pending_patches(thread, pending);

            // Skip inference if requested
            if skip_inference {
                let pending_interaction = pending_interaction_from_thread(&thread);
                if let Some(interaction) = pending_interaction.clone() {
                    yield AgentEvent::InteractionRequested {
                        interaction: interaction.clone(),
                    };
                    yield AgentEvent::Pending { interaction };
                }
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    termination: if pending_interaction.is_some() {
                        TerminationReason::PendingInteraction
                    } else {
                        TerminationReason::PluginRequested
                    },
                };
                return;
            }

            // Step boundary: starting LLM call
            let assistant_msg_id = gen_message_id();
            yield AgentEvent::StepStart { message_id: assistant_msg_id.clone() };

            // Stream LLM response with unified retry + fallback model strategy.
            let inference_span = tracing_span.unwrap_or_else(tracing::Span::none);
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
                    thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                    ensure_run_finished_delta_or_error!();
                    yield AgentEvent::RunFinish {
                        thread_id: thread.id.clone(),
                        run_id: run_id.clone(),
                        result: None,
                        termination: TerminationReason::Cancelled,
                    };
                    return;
                }
                LlmAttemptOutcome::Exhausted { last_error } => {
                    // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                    match emit_cleanup_phases_and_apply(
                        thread.clone(),
                        &tool_descriptors,
                        &config.plugins,
                        "llm_stream_start_error",
                        last_error.clone(),
                    )
                    .await
                    {
                        Ok(next_thread) => {
                            thread = next_thread;
                        }
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
                            thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                            ensure_run_finished_delta_or_error!();
                            yield AgentEvent::RunFinish {
                                thread_id: thread.id.clone(),
                                run_id: run_id.clone(),
                                result: None,
                                termination: TerminationReason::Cancelled,
                            };
                            return;
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
                        match emit_cleanup_phases_and_apply(
                            thread.clone(),
                            &tool_descriptors,
                            &config.plugins,
                            "llm_stream_event_error",
                            e.to_string(),
                        )
                        .await
                        {
                            Ok(next_thread) => {
                                thread = next_thread;
                            }
                            Err(phase_error) => {
                                terminate_stream_error!(phase_error.to_string());
                            }
                        }
                        terminate_stream_error!(e.to_string());
                    }
                }
            }

            let result = collector.finish();
            loop_state.update_from_response(&result);
            let inference_duration_ms = inference_start.elapsed().as_millis() as u64;

            yield AgentEvent::InferenceComplete {
                model: inference_model,
                usage: result.usage.clone(),
                duration_ms: inference_duration_ms,
            };

            // Phase: AfterInference (with new context)
            match emit_phase_block(
                Phase::AfterInference,
                &thread,
                &tool_descriptors,
                &config.plugins,
                |step| {
                    step.response = Some(result.clone());
                },
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

            // Add assistant message with run/step metadata.
            let step_meta = step_metadata(Some(run_id.clone()), loop_state.rounds as u32);
            let assistant_msg =
                assistant_turn_message(&result, step_meta.clone(), Some(assistant_msg_id.clone()));
            thread = reduce_thread_mutations(
                thread,
                ThreadMutationBatch::default().with_message(assistant_msg),
            );

            // Phase: StepEnd (with new context) — run plugin cleanup before yielding StepEnd
            match emit_phase_block(
                Phase::StepEnd,
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

            if let Err(e) = commit_pending_delta(
                &mut thread,
                CheckpointReason::AssistantTurnCommitted,
                false,
                &run_id,
                parent_run_id.as_deref(),
                state_committer.as_ref(),
            )
            .await
            {
                terminate_stream_error!(e.to_string());
            }

            // Step boundary: finished LLM call
            yield AgentEvent::StepEnd;

            // Check if we need to execute tools
            if !result.needs_tools() {
                if let Some(ref token) = run_cancellation_token {
                    if token.is_cancelled() {
                        thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                        ensure_run_finished_delta_or_error!();
                        yield AgentEvent::RunFinish {
                            thread_id: thread.id.clone(),
                            run_id: run_id.clone(),
                            result: None,
                            termination: TerminationReason::Cancelled,
                        };
                        return;
                    }
                }
                let stop_ctx = loop_state.to_check_context(&result, &thread);
                if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
                    thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                    ensure_run_finished_delta_or_error!();
                    yield AgentEvent::RunFinish {
                        thread_id: thread.id.clone(),
                        run_id: run_id.clone(),
                        result: None,
                        termination: TerminationReason::Stopped(reason),
                    };
                    return;
                }
                let result_value = natural_result_payload(&result.text);
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: result_value,
                    termination: TerminationReason::NaturalEnd,
                };
                return;
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
            let state = match thread.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };

            let rt_for_tools =
                match runtime_with_tool_caller_context(&thread, &state, Some(&config)) {
                Ok(rt) => rt,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };
            let sid_for_tools = thread.id.clone();
            let mut tool_future: Pin<Box<dyn Future<Output = Result<Vec<ToolExecutionResult>, AgentLoopError>> + Send>> =
                Box::pin(execute_tool_calls_with_phases(
                    &tools,
                    &result.tool_calls,
                    &state,
                    &tool_descriptors,
                    &config.plugins,
                    config.parallel_tools,
                    Some(activity_manager.clone()),
                    Some(&rt_for_tools),
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
                        thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                        ensure_run_finished_delta_or_error!();
                        yield AgentEvent::RunFinish {
                            thread_id: thread.id.clone(),
                            run_id: run_id.clone(),
                            result: None,
                            termination: TerminationReason::Cancelled,
                        };
                        return;
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
            let session_before_apply = thread.clone();
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
                    thread = session_before_apply;
                    terminate_stream_error!(e.to_string());
                }
            };
            thread = applied.thread;

            if let Err(e) = commit_pending_delta(
                &mut thread,
                CheckpointReason::ToolResultsCommitted,
                false,
                &run_id,
                parent_run_id.as_deref(),
                state_committer.as_ref(),
            )
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
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    termination: TerminationReason::PendingInteraction,
                };
                return;
            }

            // Track tool round metrics for stop condition evaluation.
            let error_count = results
                .iter()
                .filter(|r| r.execution.result.is_error())
                .count();
            loop_state.record_tool_round(&result.tool_calls, error_count);
            loop_state.rounds += 1;

            // Check stop conditions.
            let stop_ctx = loop_state.to_check_context(&result, &thread);
            if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    termination: TerminationReason::Stopped(reason),
                };
                return;
            }
        }
    })
}
