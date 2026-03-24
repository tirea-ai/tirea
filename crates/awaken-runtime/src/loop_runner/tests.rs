use std::sync::Arc;

use super::*;
use crate::hooks::PhaseContext;
use crate::phase::ExecutionEnv;
use crate::state::StateStore;
use awaken_contract::contract::content::ContentBlock;
use awaken_contract::contract::context_message::ContextMessage;
use awaken_contract::contract::message::{Message, Role};
use awaken_contract::model::{
    PendingScheduledActions, Phase, ScheduledAction, ScheduledActionEnvelope,
    ScheduledActionQueueUpdate, ScheduledActionSpec,
};

use super::actions::{
    LoopActionHandlersPlugin, apply_context_messages, apply_tool_filter_payloads, extract_actions,
    merge_override_payloads, resolve_intercept_payloads, take_context_messages,
};
use crate::agent::state::{
    AddContextMessage, ExcludeTool, IncludeOnlyTools, RunLifecycle, RunLifecycleUpdate,
    SetInferenceOverride,
};
use crate::phase::PhaseRuntime;

/// Helper: create a PhaseRuntime + ExecutionEnv with action handlers registered.
fn test_runtime() -> (PhaseRuntime, ExecutionEnv) {
    let store = StateStore::new();
    store
        .install_plugin(LoopStatePlugin)
        .expect("install LoopStatePlugin");

    // Initialize RunLifecycle so step counting works
    let mut patch = crate::state::MutationBatch::new();
    patch.update::<RunLifecycle>(RunLifecycleUpdate::Start {
        run_id: "test".into(),
        updated_at: 0,
    });
    store.commit(patch).expect("init lifecycle");

    let runtime = PhaseRuntime::new(store).expect("create runtime");
    let env =
        ExecutionEnv::from_plugins(&[Arc::new(LoopActionHandlersPlugin)], &Default::default())
            .expect("build env");
    (runtime, env)
}

/// Helper: push a context message action into the pending queue.
fn enqueue_context_message(store: &StateStore, id: u64, msg: ContextMessage) {
    let payload = AddContextMessage::encode_payload(&msg).expect("encode payload");
    let mut batch = crate::state::MutationBatch::new();
    batch.update::<PendingScheduledActions>(ScheduledActionQueueUpdate::Push(
        ScheduledActionEnvelope {
            id,
            action: ScheduledAction::new(AddContextMessage::PHASE, AddContextMessage::KEY, payload),
        },
    ));
    store.commit(batch).expect("commit enqueue");
}

/// Helper: extract all text from a message's content blocks.
fn text_of(msg: &Message) -> String {
    msg.content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// -----------------------------------------------------------------------
// apply_context_messages tests (message placement)
// -----------------------------------------------------------------------

#[test]
fn context_message_injected_at_system_target() {
    let mut messages = vec![
        Message::system("base system prompt"),
        Message::user("hello"),
    ];
    let ctx = vec![ContextMessage::system("reminder", "remember the rules")];
    apply_context_messages(&mut messages, ctx, true);

    // System-target message should be inserted after the base system prompt (index 1)
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, Role::System);
    assert_eq!(text_of(&messages[0]), "base system prompt");
    assert_eq!(messages[1].role, Role::System);
    assert_eq!(text_of(&messages[1]), "remember the rules");
    assert_eq!(messages[2].role, Role::User);
}

#[test]
fn context_message_injected_at_suffix_target() {
    let mut messages = vec![
        Message::system("system"),
        Message::user("hello"),
        Message::system(""), // simulate assistant-like; using system for simplicity
    ];
    let original_len = messages.len();
    let ctx = vec![ContextMessage::suffix_system(
        "suffix.key",
        "final reminder",
    )];
    apply_context_messages(&mut messages, ctx, true);

    // Suffix messages should be appended at the end
    assert_eq!(messages.len(), original_len + 1);
    let last = messages.last().unwrap();
    assert_eq!(last.role, Role::System);
    assert_eq!(text_of(last), "final reminder");
}

#[test]
fn multiple_context_messages_sorted_by_target() {
    let mut messages = vec![Message::system("system prompt"), Message::user("user msg")];

    let ctx = vec![
        ContextMessage::suffix_system("s1", "suffix text"),
        ContextMessage::system("sys1", "after-system text"),
        ContextMessage::conversation("conv1", Role::User, "conversation text"),
    ];
    apply_context_messages(&mut messages, ctx, true);

    // Expected order:
    // [0] system prompt (original)
    // [1] after-system text (System target, after base system prompt)
    // [2] conversation text (Conversation target, after system messages)
    // [3] user msg (original)
    // [4] suffix text (SuffixSystem target, at end)
    assert_eq!(messages.len(), 5);
    assert_eq!(text_of(&messages[0]), "system prompt");
    assert_eq!(text_of(&messages[1]), "after-system text");
    assert_eq!(text_of(&messages[2]), "conversation text");
    assert_eq!(text_of(&messages[3]), "user msg");
    assert_eq!(text_of(&messages[4]), "suffix text");
}

// -----------------------------------------------------------------------
// Handler-based context message tests (throttle logic via run_phase)
// -----------------------------------------------------------------------

#[tokio::test]
async fn throttle_zero_cooldown_always_injects() {
    let (runtime, env) = test_runtime();
    let store = runtime.store();

    for step in 0..5u32 {
        enqueue_context_message(
            store,
            step as u64,
            ContextMessage::system("always", "inject me").with_cooldown(0),
        );
        // Simulate step completion for throttle tracking
        if step > 0 {
            let mut patch = crate::state::MutationBatch::new();
            patch.update::<RunLifecycle>(RunLifecycleUpdate::StepCompleted { updated_at: 0 });
            store.commit(patch).expect("step completed");
        }
        let ctx = PhaseContext::new(Phase::BeforeInference, store.snapshot());
        runtime
            .run_phase_with_context(&env, ctx)
            .await
            .expect("run phase");
        let accepted = take_context_messages(store).expect("take");
        assert_eq!(
            accepted.len(),
            1,
            "cooldown=0 should inject at every step, failed at step {step}"
        );
    }
}

#[tokio::test]
async fn throttle_skips_within_cooldown() {
    let (runtime, env) = test_runtime();
    let store = runtime.store();

    // Step 1 (step_count=0 → current_step=1): first injection, should be accepted
    enqueue_context_message(
        store,
        1,
        ContextMessage::system("throttled", "content").with_cooldown(3),
    );
    let ctx = PhaseContext::new(Phase::BeforeInference, store.snapshot());
    runtime
        .run_phase_with_context(&env, ctx)
        .await
        .expect("step 1");
    let accepted = take_context_messages(store).expect("take step 1");
    assert_eq!(accepted.len(), 1, "first injection should pass");

    // Steps 2 and 3: within cooldown, should be skipped
    for step in 2..=3u32 {
        let mut patch = crate::state::MutationBatch::new();
        patch.update::<RunLifecycle>(RunLifecycleUpdate::StepCompleted { updated_at: 0 });
        store.commit(patch).expect("step completed");

        enqueue_context_message(
            store,
            10 + step as u64,
            ContextMessage::system("throttled", "content").with_cooldown(3),
        );
        let ctx = PhaseContext::new(Phase::BeforeInference, store.snapshot());
        runtime
            .run_phase_with_context(&env, ctx)
            .await
            .expect(&format!("step {step}"));
        let accepted = take_context_messages(store).unwrap_or_else(|e| panic!("step {step}: {e}"));
        assert_eq!(
            accepted.len(),
            0,
            "should be throttled at step {step} (cooldown=3, last_step=1)"
        );
    }

    // Step 4 (step_count=3 → current_step=4): cooldown expired (4 - 1 >= 3)
    let mut patch = crate::state::MutationBatch::new();
    patch.update::<RunLifecycle>(RunLifecycleUpdate::StepCompleted { updated_at: 0 });
    store.commit(patch).expect("step completed");

    enqueue_context_message(
        store,
        20,
        ContextMessage::system("throttled", "content").with_cooldown(3),
    );
    let ctx = PhaseContext::new(Phase::BeforeInference, store.snapshot());
    runtime
        .run_phase_with_context(&env, ctx)
        .await
        .expect("step 4");
    let accepted = take_context_messages(store).expect("take step 4");
    assert_eq!(
        accepted.len(),
        1,
        "cooldown expired at step 4, should inject"
    );
}

#[tokio::test]
async fn throttle_bypassed_on_content_change() {
    let (runtime, env) = test_runtime();
    let store = runtime.store();

    // Step 1: initial injection
    enqueue_context_message(
        store,
        1,
        ContextMessage::system("changing", "original content").with_cooldown(10),
    );
    let ctx = PhaseContext::new(Phase::BeforeInference, store.snapshot());
    runtime
        .run_phase_with_context(&env, ctx)
        .await
        .expect("step 1");
    let accepted = take_context_messages(store).expect("take step 1");
    assert_eq!(accepted.len(), 1);

    // Step 2: same content, within cooldown — should be throttled
    let mut patch = crate::state::MutationBatch::new();
    patch.update::<RunLifecycle>(RunLifecycleUpdate::StepCompleted { updated_at: 0 });
    store.commit(patch).expect("step completed");

    enqueue_context_message(
        store,
        2,
        ContextMessage::system("changing", "original content").with_cooldown(10),
    );
    let ctx = PhaseContext::new(Phase::BeforeInference, store.snapshot());
    runtime
        .run_phase_with_context(&env, ctx)
        .await
        .expect("step 2");
    let accepted = take_context_messages(store).expect("take step 2");
    assert_eq!(
        accepted.len(),
        0,
        "same content within cooldown should be throttled"
    );

    // Step 3: different content, within cooldown — should bypass
    let mut patch = crate::state::MutationBatch::new();
    patch.update::<RunLifecycle>(RunLifecycleUpdate::StepCompleted { updated_at: 0 });
    store.commit(patch).expect("step completed");

    enqueue_context_message(
        store,
        3,
        ContextMessage::system("changing", "updated content").with_cooldown(10),
    );
    let ctx = PhaseContext::new(Phase::BeforeInference, store.snapshot());
    runtime
        .run_phase_with_context(&env, ctx)
        .await
        .expect("step 3");
    let accepted = take_context_messages(store).expect("take step 3");
    assert_eq!(
        accepted.len(),
        1,
        "different content should bypass cooldown"
    );
    assert_eq!(text_of_ctx(&accepted[0]), "updated content");
}

/// Verify that tracing instrumentation does not panic when no subscriber is installed.
///
/// Exercises the context transform (which emits `truncation_applied`) and
/// direct tracing macro calls matching those added to the loop runner and engine.
#[test]
fn tracing_does_not_panic_without_subscriber() {
    use crate::context::ContextTransform;
    use awaken_contract::contract::inference::ContextWindowPolicy;
    use awaken_contract::contract::transform::InferenceRequestTransform;

    // Exercise ContextTransform truncation path (emits tracing::debug!)
    let policy = ContextWindowPolicy {
        max_context_tokens: 40,
        max_output_tokens: 0,
        min_recent_messages: 1,
        enable_prompt_cache: false,
        autocompact_threshold: None,
        compaction_mode: Default::default(),
        compaction_raw_suffix_messages: 2,
    };
    let transform = ContextTransform::new(policy);
    let mut msgs = vec![Message::system("sys")];
    for i in 0..10 {
        msgs.push(Message::user(format!("msg {i}")));
        msgs.push(Message::assistant(format!("reply {i}")));
    }
    // This triggers the truncation_applied tracing call — must not panic
    let _output = transform.transform(msgs, &[]);

    // Exercise loop-runner-style tracing macros directly — must not panic
    tracing::info!(step = 1u64, "step_start");
    tracing::info!(
        model = "test-model",
        input_tokens = 100u64,
        output_tokens = 50u64,
        duration_ms = 42u64,
        "inference_complete"
    );
    tracing::info!(
        tool_name = "calculator",
        call_id = "c1",
        outcome = "Succeeded",
        "tool_call_done"
    );
    tracing::warn!(reason = "NaturalEnd", "run_terminated");

    // Exercise engine-style tracing macros — must not panic
    tracing::debug!(phase = "StepStart", hooks = 3usize, "gather_start");
    tracing::debug!(phase = "StepStart", actions = 2usize, "execute_start");
    tracing::warn!(phase = "StepStart", "exclusive_conflict_fallback");

    // Exercise context compaction tracing — must not panic
    tracing::info!(
        pre_tokens = 2000usize,
        post_tokens = 500usize,
        boundary = 10usize,
        "compaction_complete"
    );
    tracing::debug!(dropped = 5usize, kept = 8usize, "truncation_applied");
}

/// Helper: extract text from a ContextMessage's content blocks.
fn text_of_ctx(msg: &ContextMessage) -> String {
    msg.content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// -----------------------------------------------------------------------
// Tool filter tests (extract_actions + apply_tool_filter_payloads)
// -----------------------------------------------------------------------

use awaken_contract::contract::tool::ToolDescriptor;

/// Helper: build a Vec<ScheduledAction> from ExcludeTool payloads.
fn make_exclude_actions(tool_ids: &[&str]) -> Vec<ScheduledAction> {
    tool_ids
        .iter()
        .map(|id| {
            let payload = ExcludeTool::encode_payload(&id.to_string()).expect("encode");
            ScheduledAction::new(ExcludeTool::PHASE, ExcludeTool::KEY, payload)
        })
        .collect()
}

/// Helper: build a Vec<ScheduledAction> from IncludeOnlyTools payloads.
fn make_include_actions(tool_id_lists: &[Vec<String>]) -> Vec<ScheduledAction> {
    tool_id_lists
        .iter()
        .map(|ids| {
            let payload = IncludeOnlyTools::encode_payload(ids).expect("encode");
            ScheduledAction::new(IncludeOnlyTools::PHASE, IncludeOnlyTools::KEY, payload)
        })
        .collect()
}

/// Helper: create a simple tool descriptor with the given id.
fn tool(id: &str) -> ToolDescriptor {
    ToolDescriptor::new(id, id, format!("{id} tool"))
}

#[test]
fn exclude_tool_removes_from_request() {
    let mut tools = vec![tool("search"), tool("calculator"), tool("browser")];
    let mut actions = make_exclude_actions(&["search"]);
    let exclusion_payloads = extract_actions::<ExcludeTool>(&mut actions);

    assert!(actions.is_empty(), "actions should be drained");
    apply_tool_filter_payloads(&mut tools, exclusion_payloads, vec![]);

    let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
    assert!(!ids.contains(&"search"), "search should be excluded");
    assert!(ids.contains(&"calculator"));
    assert!(ids.contains(&"browser"));
    assert_eq!(tools.len(), 2);
}

#[test]
fn include_only_tools_filters_to_subset() {
    let mut tools = vec![
        tool("search"),
        tool("calculator"),
        tool("browser"),
        tool("code_exec"),
    ];
    let mut actions = make_include_actions(&[vec!["calculator".into(), "browser".into()]]);
    let inclusion_payloads = extract_actions::<IncludeOnlyTools>(&mut actions);

    apply_tool_filter_payloads(&mut tools, vec![], inclusion_payloads);

    let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"calculator"));
    assert!(ids.contains(&"browser"));
}

#[test]
fn exclude_and_include_only_combined() {
    let mut tools = vec![tool("search"), tool("calculator"), tool("browser")];
    let mut actions = make_include_actions(&[vec!["search".into(), "calculator".into()]]);
    actions.extend(make_exclude_actions(&["search"]));

    let exclusion_payloads = extract_actions::<ExcludeTool>(&mut actions);
    let inclusion_payloads = extract_actions::<IncludeOnlyTools>(&mut actions);
    apply_tool_filter_payloads(&mut tools, exclusion_payloads, inclusion_payloads);

    let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["calculator"]);
}

#[test]
fn multiple_exclude_tool_actions() {
    let mut tools = vec![tool("a"), tool("b"), tool("c"), tool("d")];
    let mut actions = make_exclude_actions(&["a", "c"]);
    let exclusion_payloads = extract_actions::<ExcludeTool>(&mut actions);

    apply_tool_filter_payloads(&mut tools, exclusion_payloads, vec![]);

    let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["b", "d"]);
}

#[test]
fn no_filter_actions_leaves_tools_unchanged() {
    let mut tools = vec![tool("search"), tool("calculator")];
    apply_tool_filter_payloads(&mut tools, vec![], vec![]);
    assert_eq!(tools.len(), 2);
}

#[test]
fn multiple_include_only_actions_union() {
    let mut tools = vec![tool("a"), tool("b"), tool("c"), tool("d")];
    let mut actions = make_include_actions(&[vec!["a".into()], vec!["c".into()]]);
    let inclusion_payloads = extract_actions::<IncludeOnlyTools>(&mut actions);

    apply_tool_filter_payloads(&mut tools, vec![], inclusion_payloads);

    let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "c"]);
}

// -----------------------------------------------------------------------
// Inference override extraction test
// -----------------------------------------------------------------------

#[test]
fn inference_override_merges_via_extract() {
    let ovr1 = awaken_contract::contract::inference::InferenceOverride {
        model: Some("gpt-4".into()),
        temperature: Some(0.7),
        ..Default::default()
    };
    let ovr2 = awaken_contract::contract::inference::InferenceOverride {
        temperature: Some(0.9),
        max_tokens: Some(1000),
        ..Default::default()
    };

    let mut actions = vec![
        ScheduledAction::new(
            SetInferenceOverride::PHASE,
            SetInferenceOverride::KEY,
            SetInferenceOverride::encode_payload(&ovr1).expect("encode"),
        ),
        ScheduledAction::new(
            SetInferenceOverride::PHASE,
            SetInferenceOverride::KEY,
            SetInferenceOverride::encode_payload(&ovr2).expect("encode"),
        ),
    ];

    let override_payloads = extract_actions::<SetInferenceOverride>(&mut actions);
    assert!(actions.is_empty(), "actions should be drained");
    assert_eq!(override_payloads.len(), 2);

    let mut overrides = None;
    merge_override_payloads(&mut overrides, override_payloads);
    let ovr = overrides.expect("should have overrides");
    assert_eq!(ovr.model.as_deref(), Some("gpt-4"));
    assert_eq!(ovr.temperature, Some(0.9)); // last-wins
    assert_eq!(ovr.max_tokens, Some(1000));
}

// -----------------------------------------------------------------------
// Tool intercept resolution tests
// -----------------------------------------------------------------------

#[test]
fn intercept_block_wins_over_suspend() {
    use awaken_contract::contract::suspension::{
        PendingToolCall, SuspendTicket, Suspension, ToolCallResumeMode,
    };
    use awaken_contract::contract::tool_intercept::ToolInterceptPayload;

    let payloads = vec![
        ToolInterceptPayload::Suspend(SuspendTicket {
            suspension: Suspension::default(),
            pending: PendingToolCall::default(),
            resume_mode: ToolCallResumeMode::default(),
        }),
        ToolInterceptPayload::Block {
            reason: "blocked".into(),
        },
    ];
    let winner = resolve_intercept_payloads(payloads);
    assert!(matches!(winner, Some(ToolInterceptPayload::Block { .. })));
}

#[test]
fn intercept_same_priority_keeps_first() {
    use awaken_contract::contract::tool::ToolResult;
    use awaken_contract::contract::tool_intercept::ToolInterceptPayload;

    let payloads = vec![
        ToolInterceptPayload::SetResult(ToolResult::success("first", serde_json::json!({}))),
        ToolInterceptPayload::SetResult(ToolResult::success("second", serde_json::json!({}))),
    ];
    let winner = resolve_intercept_payloads(payloads);
    match winner {
        Some(ToolInterceptPayload::SetResult(r)) => assert_eq!(r.tool_name, "first"),
        other => panic!("expected SetResult, got {other:?}"),
    }
}

#[test]
fn intercept_empty_returns_none() {
    let winner = resolve_intercept_payloads(vec![]);
    assert!(winner.is_none());
}

#[test]
fn extract_actions_leaves_unrelated_actions() {
    let mut actions = vec![
        ScheduledAction::new(
            ExcludeTool::PHASE,
            ExcludeTool::KEY,
            ExcludeTool::encode_payload(&"search".to_string()).expect("encode"),
        ),
        ScheduledAction::new(
            AddContextMessage::PHASE,
            AddContextMessage::KEY,
            AddContextMessage::encode_payload(&ContextMessage::system("k", "v")).expect("encode"),
        ),
    ];

    let excluded = extract_actions::<ExcludeTool>(&mut actions);
    assert_eq!(excluded.len(), 1);
    assert_eq!(excluded[0], "search");
    // AddContextMessage action should remain
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].key, AddContextMessage::KEY);
}
