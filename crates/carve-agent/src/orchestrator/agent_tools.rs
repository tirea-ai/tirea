use super::{AgentOs, AgentRegistry};
use crate::contracts::agent_plugin::AgentPlugin;
use crate::contracts::events::AgentEvent;
use crate::contracts::phase::{Phase, StepContext};
use crate::contracts::state_types::{
    AgentRunState, AgentRunStatus, AgentState, Interaction, ToolPermissionBehavior,
    AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX, AGENT_STATE_PATH,
};
use crate::contracts::traits::tool::{Tool, ToolDescriptor, ToolResult, ToolStatus};
use crate::engine::stop_conditions::StopReason;
use crate::engine::tool_filter::{
    is_runtime_allowed, RUNTIME_ALLOWED_AGENTS_KEY, RUNTIME_EXCLUDED_AGENTS_KEY,
};
use crate::extensions::permission::PermissionContextExt;
pub(crate) use crate::runtime::loop_runner::TOOL_RUNTIME_CALLER_AGENT_ID_KEY as RUNTIME_CALLER_AGENT_ID_KEY;
use crate::runtime::loop_runner::{
    ChannelStateCommitter, RunCancellationToken, RunContext, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
use crate::types::{Message, Role, ToolCall};
use async_trait::async_trait;
use carve_state::Context;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

const RUNTIME_CALLER_SESSION_ID_KEY: &str = TOOL_RUNTIME_CALLER_THREAD_ID_KEY;
const RUNTIME_CALLER_STATE_KEY: &str = TOOL_RUNTIME_CALLER_STATE_KEY;
const RUNTIME_CALLER_MESSAGES_KEY: &str = TOOL_RUNTIME_CALLER_MESSAGES_KEY;
const RUNTIME_RUN_ID_KEY: &str = "run_id";
const RUNTIME_PARENT_RUN_ID_KEY: &str = "parent_run_id";

#[derive(Debug, Clone)]
pub struct AgentRunSummary {
    pub run_id: String,
    pub target_agent_id: String,
    pub status: AgentRunStatus,
    pub assistant: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct AgentRunRecord {
    epoch: u64,
    owner_thread_id: String,
    target_agent_id: String,
    parent_run_id: Option<String>,
    status: AgentRunStatus,
    thread: crate::thread::Thread,
    assistant: Option<String>,
    error: Option<String>,
    run_cancellation_requested: bool,
    cancellation_token: Option<RunCancellationToken>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentRunManager {
    runs: Arc<Mutex<HashMap<String, AgentRunRecord>>>,
}

impl AgentRunManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get_owned_summary(
        &self,
        owner_thread_id: &str,
        run_id: &str,
    ) -> Option<AgentRunSummary> {
        let runs = self.runs.lock().await;
        let rec = runs.get(run_id)?;
        if rec.owner_thread_id != owner_thread_id {
            return None;
        }
        Some(AgentRunSummary {
            run_id: run_id.to_string(),
            target_agent_id: rec.target_agent_id.clone(),
            status: rec.status,
            assistant: rec.assistant.clone(),
            error: rec.error.clone(),
        })
    }

    pub async fn running_or_stopped_for_owner(
        &self,
        owner_thread_id: &str,
    ) -> Vec<AgentRunSummary> {
        let runs = self.runs.lock().await;
        let mut out: Vec<AgentRunSummary> = runs
            .iter()
            .filter_map(|(run_id, rec)| {
                if rec.owner_thread_id != owner_thread_id {
                    return None;
                }
                match rec.status {
                    AgentRunStatus::Running | AgentRunStatus::Stopped => Some(AgentRunSummary {
                        run_id: run_id.clone(),
                        target_agent_id: rec.target_agent_id.clone(),
                        status: rec.status,
                        assistant: rec.assistant.clone(),
                        error: rec.error.clone(),
                    }),
                    _ => None,
                }
            })
            .collect();
        out.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        out
    }

    pub async fn all_for_owner(&self, owner_thread_id: &str) -> Vec<AgentRunSummary> {
        let runs = self.runs.lock().await;
        let mut out: Vec<AgentRunSummary> = runs
            .iter()
            .filter_map(|(run_id, rec)| {
                if rec.owner_thread_id != owner_thread_id {
                    return None;
                }
                Some(AgentRunSummary {
                    run_id: run_id.clone(),
                    target_agent_id: rec.target_agent_id.clone(),
                    status: rec.status,
                    assistant: rec.assistant.clone(),
                    error: rec.error.clone(),
                })
            })
            .collect();
        out.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        out
    }

    async fn owned_record(
        &self,
        owner_thread_id: &str,
        run_id: &str,
    ) -> Option<crate::thread::Thread> {
        let runs = self.runs.lock().await;
        let rec = runs.get(run_id)?;
        if rec.owner_thread_id != owner_thread_id {
            return None;
        }
        Some(rec.thread.clone())
    }

    pub async fn stop_owned_tree(
        &self,
        owner_thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<AgentRunSummary>, String> {
        let mut runs = self.runs.lock().await;
        let Some(root_status) = runs.get(run_id).map(|r| r.status) else {
            return Err(format!("Unknown run_id: {run_id}"));
        };
        if runs
            .get(run_id)
            .is_some_and(|r| r.owner_thread_id != owner_thread_id)
        {
            return Err(format!("Unknown run_id: {run_id}"));
        }

        let run_ids = collect_descendant_run_ids_by_parent(&runs, owner_thread_id, run_id, true);
        if run_ids.is_empty() {
            return Err(format!(
                "Run '{run_id}' is not running (current status: {})",
                root_status.as_str()
            ));
        }

        let mut stopped = false;
        let mut out = Vec::with_capacity(run_ids.len());
        for id in run_ids {
            if let Some(rec) = runs.get_mut(&id) {
                if rec.status == AgentRunStatus::Running {
                    rec.run_cancellation_requested = true;
                    rec.status = AgentRunStatus::Stopped;
                    stopped = true;
                    if let Some(token) = rec.cancellation_token.take() {
                        token.cancel();
                    }
                }
                out.push(AgentRunSummary {
                    run_id: id,
                    target_agent_id: rec.target_agent_id.clone(),
                    status: rec.status,
                    assistant: rec.assistant.clone(),
                    error: rec.error.clone(),
                });
            }
        }

        if stopped {
            return Ok(out);
        }

        Err(format!(
            "Run '{run_id}' is not running (current status: {})",
            root_status.as_str()
        ))
    }

    async fn put_running(
        &self,
        run_id: &str,
        owner_thread_id: String,
        target_agent_id: String,
        parent_run_id: Option<String>,
        thread: crate::thread::Thread,
        cancellation_token: Option<RunCancellationToken>,
    ) -> u64 {
        let mut runs = self.runs.lock().await;
        let epoch = runs.get(run_id).map(|r| r.epoch + 1).unwrap_or(1);
        runs.insert(
            run_id.to_string(),
            AgentRunRecord {
                epoch,
                owner_thread_id,
                target_agent_id,
                parent_run_id,
                status: AgentRunStatus::Running,
                thread,
                assistant: None,
                error: None,
                run_cancellation_requested: false,
                cancellation_token,
            },
        );
        epoch
    }

    async fn update_after_completion(
        &self,
        run_id: &str,
        epoch: u64,
        completion: AgentRunCompletion,
    ) -> Option<AgentRunSummary> {
        let mut runs = self.runs.lock().await;
        let rec = runs.get_mut(run_id)?;
        if rec.epoch != epoch {
            // Stale completion from a previous generation (e.g. stopped run that was resumed).
            return None;
        }
        rec.thread = completion.thread;
        rec.assistant = completion.assistant;
        rec.error = completion.error;

        // Explicit run-cancellation request wins over terminal status from executor.
        rec.status = if rec.run_cancellation_requested {
            AgentRunStatus::Stopped
        } else {
            completion.status
        };
        rec.cancellation_token = None;

        Some(AgentRunSummary {
            run_id: run_id.to_string(),
            target_agent_id: rec.target_agent_id.clone(),
            status: rec.status,
            assistant: rec.assistant.clone(),
            error: rec.error.clone(),
        })
    }

    async fn record_for_resume(
        &self,
        owner_thread_id: &str,
        run_id: &str,
    ) -> Result<AgentRunRecord, String> {
        let runs = self.runs.lock().await;
        let Some(rec) = runs.get(run_id) else {
            return Err(format!("Unknown run_id: {run_id}"));
        };
        if rec.owner_thread_id != owner_thread_id {
            return Err(format!("Unknown run_id: {run_id}"));
        }
        Ok(rec.clone())
    }
}

#[derive(Debug)]
struct AgentRunCompletion {
    thread: crate::thread::Thread,
    status: AgentRunStatus,
    assistant: Option<String>,
    error: Option<String>,
}

fn last_assistant_message(thread: &crate::thread::Thread) -> Option<String> {
    thread
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.content.clone())
}

async fn execute_target_agent(
    os: AgentOs,
    target_agent_id: String,
    thread: crate::thread::Thread,
    cancellation_token: Option<RunCancellationToken>,
) -> AgentRunCompletion {
    let (checkpoint_tx, mut checkpoints) = tokio::sync::mpsc::unbounded_channel();
    let run_ctx = RunContext {
        cancellation_token,
        ..RunContext::default()
    }
    .with_state_committer(Arc::new(ChannelStateCommitter::new(checkpoint_tx)));
    let mut events = match os.run_stream_with_context(&target_agent_id, thread.clone(), run_ctx) {
        Ok(stream) => stream,
        Err(e) => {
            return AgentRunCompletion {
                thread,
                status: AgentRunStatus::Failed,
                assistant: None,
                error: Some(e.to_string()),
            };
        }
    };

    let mut saw_error: Option<String> = None;
    let mut stop_reason: Option<StopReason> = None;
    let mut final_thread = thread.clone();
    let mut checkpoints_open = true;

    while let Some(ev) = events.next().await {
        match ev {
            AgentEvent::Error { message } => {
                if saw_error.is_none() {
                    saw_error = Some(message);
                }
            }
            AgentEvent::RunFinish {
                stop_reason: reason,
                ..
            } => {
                stop_reason = reason;
            }
            _ => {}
        }

        while let Ok(delta) = checkpoints.try_recv() {
            delta.apply_to(&mut final_thread);
        }
    }

    while checkpoints_open {
        match checkpoints.recv().await {
            Some(delta) => delta.apply_to(&mut final_thread),
            None => checkpoints_open = false,
        }
    }

    let assistant = last_assistant_message(&final_thread);

    if saw_error.is_some() {
        return AgentRunCompletion {
            thread: final_thread,
            status: AgentRunStatus::Failed,
            assistant,
            error: saw_error,
        };
    }

    let status = match stop_reason {
        Some(StopReason::Cancelled) => AgentRunStatus::Stopped,
        _ => AgentRunStatus::Completed,
    };

    AgentRunCompletion {
        thread: final_thread,
        status,
        assistant,
        error: None,
    }
}

fn to_tool_result(tool_name: &str, summary: AgentRunSummary) -> ToolResult {
    ToolResult::success(
        tool_name,
        json!({
            "run_id": summary.run_id,
            "agent_id": summary.target_agent_id,
            "status": summary.status.as_str(),
            "assistant": summary.assistant,
            "error": summary.error,
        }),
    )
}

fn tool_error(tool_name: &str, code: &str, message: impl Into<String>) -> ToolResult {
    let message = message.into();
    ToolResult {
        tool_name: tool_name.to_string(),
        status: ToolStatus::Error,
        data: json!({
            "error": {
                "code": code,
                "message": message,
            }
        }),
        message: Some(format!("[{code}] {message}")),
        metadata: HashMap::new(),
    }
}

fn runtime_run_id(runtime: Option<&carve_state::Runtime>) -> Option<String> {
    runtime
        .and_then(|rt| rt.value(RUNTIME_RUN_ID_KEY))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn bind_child_lineage(
    mut thread: crate::thread::Thread,
    run_id: &str,
    parent_run_id: Option<&str>,
    parent_thread_id: Option<&str>,
) -> crate::thread::Thread {
    if thread.parent_thread_id.is_none() {
        thread.parent_thread_id = parent_thread_id.map(str::to_string);
    }
    let current_run_id = thread
        .runtime
        .value(RUNTIME_RUN_ID_KEY)
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let current_parent_run_id = thread
        .runtime
        .value(RUNTIME_PARENT_RUN_ID_KEY)
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let parent_mismatch = match (current_parent_run_id.as_deref(), parent_run_id) {
        (Some(cur), Some(expected)) => cur != expected,
        (Some(_), None) => true,
        _ => false,
    };
    let run_mismatch = current_run_id.as_deref().is_some_and(|cur| cur != run_id);

    if run_mismatch || parent_mismatch {
        thread = thread.with_runtime(carve_state::Runtime::new());
    }

    if thread.runtime.value(RUNTIME_RUN_ID_KEY).is_none() {
        let _ = thread.runtime.set(RUNTIME_RUN_ID_KEY, run_id);
    }
    if let Some(parent_run_id) = parent_run_id {
        if thread.runtime.value(RUNTIME_PARENT_RUN_ID_KEY).is_none() {
            let _ = thread.runtime.set(RUNTIME_PARENT_RUN_ID_KEY, parent_run_id);
        }
    }
    thread
}

fn parent_run_id_from_thread(thread: Option<&crate::thread::Thread>) -> Option<String> {
    thread
        .and_then(|s| s.runtime.value(RUNTIME_PARENT_RUN_ID_KEY))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn as_agent_run_state(
    summary: &AgentRunSummary,
    thread: Option<crate::thread::Thread>,
) -> AgentRunState {
    let parent_run_id = parent_run_id_from_thread(thread.as_ref());
    AgentRunState {
        run_id: summary.run_id.clone(),
        parent_run_id,
        target_agent_id: summary.target_agent_id.clone(),
        status: summary.status,
        assistant: summary.assistant.clone(),
        error: summary.error.clone(),
        thread,
    }
}

fn as_agent_run_summary(run_id: &str, state: &AgentRunState) -> AgentRunSummary {
    AgentRunSummary {
        run_id: run_id.to_string(),
        target_agent_id: state.target_agent_id.clone(),
        status: state.status,
        assistant: state.assistant.clone(),
        error: state.error.clone(),
    }
}

fn set_persisted_run(ctx: &Context<'_>, run_id: &str, run: AgentRunState) {
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.agent_runs_insert(run_id.to_string(), run);
}

fn parse_persisted_runs(ctx: &Context<'_>) -> HashMap<String, AgentRunState> {
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.agent_runs().ok().unwrap_or_default()
}

fn parse_persisted_runs_from_doc(doc: &Value) -> HashMap<String, AgentRunState> {
    doc.get(AGENT_STATE_PATH)
        .and_then(|v| v.get("agent_runs"))
        .cloned()
        .and_then(|v| serde_json::from_value::<HashMap<String, AgentRunState>>(v).ok())
        .unwrap_or_default()
}

fn make_orphaned_running_state(run: &AgentRunState) -> AgentRunState {
    let mut next = run.clone();
    next.status = AgentRunStatus::Stopped;
    next.error = Some("No live executor found in current process; marked stopped".to_string());
    next
}

fn collect_descendant_run_ids_by_parent(
    runs: &HashMap<String, AgentRunRecord>,
    owner_thread_id: &str,
    root_run_id: &str,
    include_root: bool,
) -> Vec<String> {
    let owned = runs
        .iter()
        .filter(|(_, rec)| rec.owner_thread_id == owner_thread_id)
        .collect::<Vec<_>>();
    if !owned.iter().any(|(id, _)| id.as_str() == root_run_id) {
        return Vec::new();
    }

    let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
    for (run_id, rec) in owned.iter() {
        if let Some(parent_run_id) = &rec.parent_run_id {
            children_by_parent
                .entry(parent_run_id.clone())
                .or_default()
                .push((*run_id).clone());
        }
    }

    let mut queue = VecDeque::from([root_run_id.to_string()]);
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if include_root || id != root_run_id {
            out.push(id.clone());
        }
        if let Some(children) = children_by_parent.get(&id) {
            for child_id in children {
                queue.push_back(child_id.clone());
            }
        }
    }
    out
}

fn collect_descendant_run_ids_from_state(
    runs: &HashMap<String, AgentRunState>,
    root_run_id: &str,
    include_root: bool,
) -> Vec<String> {
    if !runs.contains_key(root_run_id) {
        return Vec::new();
    }

    let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
    for (run_id, run) in runs.iter() {
        if let Some(parent_run_id) = &run.parent_run_id {
            children_by_parent
                .entry(parent_run_id.clone())
                .or_default()
                .push(run_id.clone());
        }
    }

    let mut queue = VecDeque::from([root_run_id.to_string()]);
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if include_root || id != root_run_id {
            out.push(id.clone());
        }
        if let Some(children) = children_by_parent.get(&id) {
            for child_id in children {
                queue.push_back(child_id.clone());
            }
        }
    }
    out
}

fn recovery_interaction_id(run_id: &str) -> String {
    format!("{AGENT_RECOVERY_INTERACTION_PREFIX}{run_id}")
}

fn build_recovery_interaction(run_id: &str, run: &AgentRunState) -> Interaction {
    Interaction::new(
        recovery_interaction_id(run_id),
        AGENT_RECOVERY_INTERACTION_ACTION,
    )
    .with_message(format!(
        "Detected interrupted run '{run_id}' (agent '{}'). Resume now?",
        run.target_agent_id
    ))
    .with_parameters(json!({
        "run_id": run_id,
        "agent_id": run.target_agent_id,
        "background": false
    }))
    .with_response_schema(json!({
        "type": "boolean"
    }))
}

fn parse_pending_interaction_from_state(state: &Value) -> Option<Interaction> {
    state
        .get(AGENT_STATE_PATH)
        .and_then(|a| a.get("pending_interaction"))
        .cloned()
        .and_then(|v| serde_json::from_value::<Interaction>(v).ok())
}

fn parse_replay_tool_calls_from_state(state: &Value) -> Vec<ToolCall> {
    state
        .get(AGENT_STATE_PATH)
        .and_then(|a| a.get("replay_tool_calls"))
        .cloned()
        .and_then(|v| serde_json::from_value::<Vec<ToolCall>>(v).ok())
        .unwrap_or_default()
}

fn set_pending_interaction_patch(
    state: &Value,
    interaction: Interaction,
    call_id: &str,
) -> Option<carve_state::TrackedPatch> {
    let ctx = Context::new(state, call_id, "agent_recovery");
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.set_pending_interaction(Some(interaction));
    let patch = ctx.take_patch();
    if patch.patch().is_empty() {
        None
    } else {
        Some(patch)
    }
}

fn set_replay_tool_calls_patch(
    state: &Value,
    replay_calls: Vec<ToolCall>,
    call_id: &str,
) -> Option<carve_state::TrackedPatch> {
    let ctx = Context::new(state, call_id, "agent_recovery");
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.set_replay_tool_calls(replay_calls);
    let patch = ctx.take_patch();
    if patch.patch().is_empty() {
        None
    } else {
        Some(patch)
    }
}

fn schedule_recovery_replay(state: &Value, step: &mut StepContext<'_>, run_id: &str) {
    let mut replay_calls = parse_replay_tool_calls_from_state(state);

    let exists = replay_calls.iter().any(|call| {
        call.name == "agent_run"
            && call
                .arguments
                .get("run_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == run_id)
    });
    if exists {
        return;
    }

    replay_calls.push(ToolCall::new(
        format!("agent_recovery_resume_{run_id}"),
        "agent_run",
        json!({
            "run_id": run_id,
            "background": false
        }),
    ));
    if let Some(patch) = set_replay_tool_calls_patch(
        state,
        replay_calls,
        &format!("agent_recovery_replay_{run_id}"),
    ) {
        step.pending_patches.push(patch);
    }
}

#[derive(Debug, Default, Clone)]
struct ReconcileOutcome {
    changed: bool,
    orphaned_run_ids: Vec<String>,
}

fn set_agent_runs_patch_from_state_doc(
    state: &Value,
    next_runs: HashMap<String, AgentRunState>,
    call_id: &str,
) -> Option<carve_state::TrackedPatch> {
    let ctx = Context::new(state, call_id, "agent_tools");
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.set_agent_runs(next_runs);
    let patch = ctx.take_patch();
    if patch.patch().is_empty() {
        None
    } else {
        Some(patch)
    }
}

async fn reconcile_persisted_runs(
    manager: &AgentRunManager,
    owner_thread_id: &str,
    runs: &mut HashMap<String, AgentRunState>,
) -> ReconcileOutcome {
    let summaries = manager.all_for_owner(owner_thread_id).await;
    let mut by_id: HashMap<String, AgentRunSummary> = HashMap::new();
    for summary in summaries {
        by_id.insert(summary.run_id.clone(), summary);
    }

    let mut changed = false;
    let mut orphaned_run_ids = Vec::new();
    let mut known_ids: Vec<String> = runs.keys().cloned().collect();
    known_ids.sort();

    for run_id in known_ids {
        let Some(current) = runs.get(&run_id).cloned() else {
            continue;
        };
        if let Some(summary) = by_id.get(&run_id) {
            let thread = manager.owned_record(owner_thread_id, &run_id).await;
            let mut next = as_agent_run_state(summary, thread.or_else(|| current.thread.clone()));
            if next.parent_run_id.is_none() {
                next.parent_run_id = current.parent_run_id.clone();
            }
            if current.status != next.status
                || current.assistant != next.assistant
                || current.error != next.error
                || current.parent_run_id != next.parent_run_id
                || current.thread.as_ref().map(|s| &s.id) != next.thread.as_ref().map(|s| &s.id)
            {
                runs.insert(run_id.clone(), next);
                changed = true;
            }
            continue;
        }

        if current.status == AgentRunStatus::Running {
            runs.insert(run_id.clone(), make_orphaned_running_state(&current));
            changed = true;
            orphaned_run_ids.push(run_id);
        }
    }

    for (run_id, summary) in by_id {
        if runs.contains_key(&run_id) {
            continue;
        }
        let thread = manager.owned_record(owner_thread_id, &run_id).await;
        runs.insert(run_id.clone(), as_agent_run_state(&summary, thread));
        changed = true;
    }

    ReconcileOutcome {
        changed,
        orphaned_run_ids,
    }
}

fn required_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn required_string(args: &Value, key: &str, tool_name: &str) -> Result<String, ToolResult> {
    optional_string(args, key)
        .ok_or_else(|| tool_error(tool_name, "invalid_arguments", format!("missing '{key}'")))
}

fn parse_caller_messages(runtime: Option<&carve_state::Runtime>) -> Option<Vec<Message>> {
    let value = runtime.and_then(|rt| rt.value(RUNTIME_CALLER_MESSAGES_KEY))?;
    serde_json::from_value::<Vec<Message>>(value.clone()).ok()
}

fn filtered_fork_messages(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .filter(|m| m.visibility == crate::types::Visibility::All)
        .filter(|m| matches!(m.role, Role::System | Role::User | Role::Assistant))
        .map(|mut m| {
            if m.role == Role::Assistant {
                m.tool_calls = None;
            }
            m.tool_call_id = None;
            m
        })
        .collect()
}

fn is_target_agent_visible(
    registry: &dyn AgentRegistry,
    target: &str,
    caller: Option<&str>,
    runtime: Option<&carve_state::Runtime>,
) -> bool {
    if caller.is_some_and(|c| c == target) {
        return false;
    }
    if !is_runtime_allowed(
        runtime,
        target,
        RUNTIME_ALLOWED_AGENTS_KEY,
        RUNTIME_EXCLUDED_AGENTS_KEY,
    ) {
        return false;
    }
    registry.get(target).is_some()
}

#[derive(Debug, Clone)]
pub struct AgentRunTool {
    os: AgentOs,
    manager: Arc<AgentRunManager>,
}

impl AgentRunTool {
    pub fn new(os: AgentOs, manager: Arc<AgentRunManager>) -> Self {
        Self { os, manager }
    }
}

#[async_trait]
impl Tool for AgentRunTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "agent_run",
            "Agent Run",
            "Run or resume a registry agent; can run in background",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Target agent id (required for new runs)" },
                "prompt": { "type": "string", "description": "Input for the target agent" },
                "run_id": { "type": "string", "description": "Existing run id to resume or inspect" },
                "fork_context": { "type": "boolean", "description": "Whether to fork caller state/messages into the new run" },
                "background": { "type": "boolean", "description": "true: run in background; false: wait for completion" }
            }
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &carve_state::Context<'_>,
    ) -> Result<ToolResult, crate::contracts::traits::tool::ToolError> {
        let tool_name = "agent_run";
        let run_id = optional_string(&args, "run_id");
        let background = required_bool(&args, "background", false);
        let fork_context = required_bool(&args, "fork_context", false);

        let runtime = ctx.runtime_ref();
        let owner_thread_id = runtime
            .and_then(|rt| rt.value(RUNTIME_CALLER_SESSION_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(owner_thread_id) = owner_thread_id else {
            return Ok(tool_error(
                tool_name,
                "missing_runtime",
                "missing caller thread context",
            ));
        };
        let caller_agent_id = runtime
            .and_then(|rt| rt.value(RUNTIME_CALLER_AGENT_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let caller_run_id = runtime_run_id(runtime);

        if let Some(run_id) = run_id {
            if let Some(existing) = self
                .manager
                .get_owned_summary(&owner_thread_id, &run_id)
                .await
            {
                match existing.status {
                    AgentRunStatus::Running
                    | AgentRunStatus::Completed
                    | AgentRunStatus::Failed => {
                        let thread = self.manager.owned_record(&owner_thread_id, &run_id).await;
                        set_persisted_run(ctx, &run_id, as_agent_run_state(&existing, thread));
                        return Ok(to_tool_result(tool_name, existing));
                    }
                    AgentRunStatus::Stopped => {
                        let mut record = match self
                            .manager
                            .record_for_resume(&owner_thread_id, &run_id)
                            .await
                        {
                            Ok(v) => v,
                            Err(e) => return Ok(tool_error(tool_name, "unknown_run", e)),
                        };

                        if !is_target_agent_visible(
                            self.os.agents_registry().as_ref(),
                            &record.target_agent_id,
                            caller_agent_id.as_deref(),
                            runtime,
                        ) {
                            return Ok(tool_error(
                                tool_name,
                                "unknown_agent",
                                format!(
                                    "Unknown or unavailable agent_id: {}",
                                    record.target_agent_id
                                ),
                            ));
                        }

                        record.thread = bind_child_lineage(
                            record.thread,
                            &run_id,
                            caller_run_id.as_deref(),
                            Some(&owner_thread_id),
                        );

                        if let Some(prompt) = optional_string(&args, "prompt") {
                            record.thread = record.thread.with_message(Message::user(prompt));
                        }

                        if background {
                            let token = RunCancellationToken::new();
                            let epoch = self
                                .manager
                                .put_running(
                                    &run_id,
                                    owner_thread_id.clone(),
                                    record.target_agent_id.clone(),
                                    caller_run_id.clone(),
                                    record.thread.clone(),
                                    Some(token.clone()),
                                )
                                .await;
                            let manager = self.manager.clone();
                            let os = self.os.clone();
                            let run_id_bg = run_id.clone();
                            let agent_id_bg = record.target_agent_id.clone();
                            let thread_bg = record.thread.clone();
                            tokio::spawn(async move {
                                let completion =
                                    execute_target_agent(os, agent_id_bg, thread_bg, Some(token))
                                        .await;
                                let _ = manager
                                    .update_after_completion(&run_id_bg, epoch, completion)
                                    .await;
                            });

                            let summary = self
                                .manager
                                .get_owned_summary(&owner_thread_id, &run_id)
                                .await
                                .expect("summary must exist right after put_running");
                            set_persisted_run(
                                ctx,
                                &run_id,
                                AgentRunState {
                                    run_id: run_id.clone(),
                                    parent_run_id: caller_run_id.clone(),
                                    target_agent_id: record.target_agent_id.clone(),
                                    status: AgentRunStatus::Running,
                                    assistant: None,
                                    error: None,
                                    thread: Some(record.thread),
                                },
                            );
                            return Ok(to_tool_result(tool_name, summary));
                        }

                        let epoch = self
                            .manager
                            .put_running(
                                &run_id,
                                owner_thread_id.clone(),
                                record.target_agent_id.clone(),
                                caller_run_id.clone(),
                                record.thread.clone(),
                                None,
                            )
                            .await;
                        let completion = execute_target_agent(
                            self.os.clone(),
                            record.target_agent_id.clone(),
                            record.thread.clone(),
                            None,
                        )
                        .await;
                        let completion_state = AgentRunState {
                            run_id: run_id.clone(),
                            parent_run_id: caller_run_id.clone(),
                            target_agent_id: record.target_agent_id,
                            status: completion.status,
                            assistant: completion.assistant.clone(),
                            error: completion.error.clone(),
                            thread: Some(completion.thread.clone()),
                        };
                        let summary = self
                            .manager
                            .update_after_completion(&run_id, epoch, completion)
                            .await
                            .expect("summary must exist after completion update");
                        set_persisted_run(ctx, &run_id, completion_state);
                        return Ok(to_tool_result(tool_name, summary));
                    }
                }
            }

            let Some(mut persisted) = parse_persisted_runs(ctx).remove(&run_id) else {
                return Ok(tool_error(
                    tool_name,
                    "unknown_run",
                    format!("Unknown run_id: {run_id}"),
                ));
            };

            let orphaned_running = persisted.status == AgentRunStatus::Running;
            if orphaned_running {
                persisted = make_orphaned_running_state(&persisted);
                set_persisted_run(ctx, &run_id, persisted.clone());
                return Ok(to_tool_result(
                    tool_name,
                    as_agent_run_summary(&run_id, &persisted),
                ));
            }

            match persisted.status {
                AgentRunStatus::Running | AgentRunStatus::Completed | AgentRunStatus::Failed => {
                    return Ok(to_tool_result(
                        tool_name,
                        as_agent_run_summary(&run_id, &persisted),
                    ));
                }
                AgentRunStatus::Stopped => {
                    if !is_target_agent_visible(
                        self.os.agents_registry().as_ref(),
                        &persisted.target_agent_id,
                        caller_agent_id.as_deref(),
                        runtime,
                    ) {
                        return Ok(tool_error(
                            tool_name,
                            "unknown_agent",
                            format!(
                                "Unknown or unavailable agent_id: {}",
                                persisted.target_agent_id
                            ),
                        ));
                    }

                    let mut child_thread = match persisted.thread {
                        Some(s) => s,
                        None => {
                            return Ok(tool_error(
                                tool_name,
                                "invalid_state",
                                format!("Run '{run_id}' cannot be resumed: missing child thread"),
                            ))
                        }
                    };
                    child_thread = bind_child_lineage(
                        child_thread,
                        &run_id,
                        caller_run_id.as_deref(),
                        Some(&owner_thread_id),
                    );

                    if let Some(prompt) = optional_string(&args, "prompt") {
                        child_thread = child_thread.with_message(Message::user(prompt));
                    }

                    if background {
                        let token = RunCancellationToken::new();
                        let epoch = self
                            .manager
                            .put_running(
                                &run_id,
                                owner_thread_id.clone(),
                                persisted.target_agent_id.clone(),
                                caller_run_id.clone(),
                                child_thread.clone(),
                                Some(token.clone()),
                            )
                            .await;
                        let manager = self.manager.clone();
                        let os = self.os.clone();
                        let run_id_bg = run_id.clone();
                        let agent_id_bg = persisted.target_agent_id.clone();
                        tokio::spawn(async move {
                            let completion =
                                execute_target_agent(os, agent_id_bg, child_thread, Some(token))
                                    .await;
                            let _ = manager
                                .update_after_completion(&run_id_bg, epoch, completion)
                                .await;
                        });

                        let summary = self
                            .manager
                            .get_owned_summary(&owner_thread_id, &run_id)
                            .await
                            .expect("summary must exist right after put_running");
                        set_persisted_run(
                            ctx,
                            &run_id,
                            AgentRunState {
                                run_id: run_id.clone(),
                                parent_run_id: caller_run_id.clone(),
                                target_agent_id: persisted.target_agent_id,
                                status: AgentRunStatus::Running,
                                assistant: None,
                                error: None,
                                thread: self.manager.owned_record(&owner_thread_id, &run_id).await,
                            },
                        );
                        return Ok(to_tool_result(tool_name, summary));
                    }

                    let epoch = self
                        .manager
                        .put_running(
                            &run_id,
                            owner_thread_id.clone(),
                            persisted.target_agent_id.clone(),
                            caller_run_id.clone(),
                            child_thread.clone(),
                            None,
                        )
                        .await;
                    let completion = execute_target_agent(
                        self.os.clone(),
                        persisted.target_agent_id.clone(),
                        child_thread,
                        None,
                    )
                    .await;
                    let completion_state = AgentRunState {
                        run_id: run_id.clone(),
                        parent_run_id: caller_run_id.clone(),
                        target_agent_id: persisted.target_agent_id,
                        status: completion.status,
                        assistant: completion.assistant.clone(),
                        error: completion.error.clone(),
                        thread: Some(completion.thread.clone()),
                    };
                    let summary = self
                        .manager
                        .update_after_completion(&run_id, epoch, completion)
                        .await
                        .expect("summary must exist after completion update");
                    set_persisted_run(ctx, &run_id, completion_state);
                    return Ok(to_tool_result(tool_name, summary));
                }
            }
        }

        let target_agent_id = match required_string(&args, "agent_id", tool_name) {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };
        let prompt = match required_string(&args, "prompt", tool_name) {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };

        if !is_target_agent_visible(
            self.os.agents_registry().as_ref(),
            &target_agent_id,
            caller_agent_id.as_deref(),
            runtime,
        ) {
            return Ok(tool_error(
                tool_name,
                "unknown_agent",
                format!("Unknown or unavailable agent_id: {target_agent_id}"),
            ));
        }

        let run_id = uuid::Uuid::now_v7().to_string();
        let thread_id = format!("agent-run-{run_id}");

        let mut child_thread = if fork_context {
            let fork_state = runtime
                .and_then(|rt| rt.value(RUNTIME_CALLER_STATE_KEY))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut forked = crate::thread::Thread::with_initial_state(thread_id, fork_state);
            if let Some(messages) = parse_caller_messages(runtime) {
                forked = forked.with_messages(filtered_fork_messages(messages));
            }
            forked
        } else {
            crate::thread::Thread::new(thread_id)
        };
        child_thread = child_thread.with_message(Message::user(prompt));
        child_thread = bind_child_lineage(
            child_thread,
            &run_id,
            caller_run_id.as_deref(),
            Some(&owner_thread_id),
        );

        if background {
            let token = RunCancellationToken::new();
            let epoch = self
                .manager
                .put_running(
                    &run_id,
                    owner_thread_id.clone(),
                    target_agent_id.clone(),
                    caller_run_id.clone(),
                    child_thread.clone(),
                    Some(token.clone()),
                )
                .await;
            let manager = self.manager.clone();
            let os = self.os.clone();
            let run_id_bg = run_id.clone();
            let target_agent_id_bg = target_agent_id.clone();
            let child_thread_bg = child_thread.clone();
            tokio::spawn(async move {
                let completion =
                    execute_target_agent(os, target_agent_id_bg, child_thread_bg, Some(token))
                        .await;
                let _ = manager
                    .update_after_completion(&run_id_bg, epoch, completion)
                    .await;
            });

            let summary = self
                .manager
                .get_owned_summary(&owner_thread_id, &run_id)
                .await
                .expect("summary must exist right after put_running");
            set_persisted_run(
                ctx,
                &run_id,
                AgentRunState {
                    run_id: run_id.clone(),
                    parent_run_id: caller_run_id.clone(),
                    target_agent_id: target_agent_id.clone(),
                    status: AgentRunStatus::Running,
                    assistant: None,
                    error: None,
                    thread: Some(child_thread),
                },
            );
            return Ok(to_tool_result(tool_name, summary));
        }

        let epoch = self
            .manager
            .put_running(
                &run_id,
                owner_thread_id.clone(),
                target_agent_id.clone(),
                caller_run_id.clone(),
                child_thread.clone(),
                None,
            )
            .await;
        let completion =
            execute_target_agent(self.os.clone(), target_agent_id.clone(), child_thread, None)
                .await;
        let completion_state = AgentRunState {
            run_id: run_id.clone(),
            parent_run_id: caller_run_id,
            target_agent_id,
            status: completion.status,
            assistant: completion.assistant.clone(),
            error: completion.error.clone(),
            thread: Some(completion.thread.clone()),
        };
        let summary = self
            .manager
            .update_after_completion(&run_id, epoch, completion)
            .await
            .expect("summary must exist after completion update");
        set_persisted_run(ctx, &run_id, completion_state);
        Ok(to_tool_result(tool_name, summary))
    }
}

#[derive(Debug, Clone)]
pub struct AgentStopTool {
    manager: Arc<AgentRunManager>,
}

impl AgentStopTool {
    pub fn new(manager: Arc<AgentRunManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for AgentStopTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "agent_stop",
            "Agent Stop",
            "Stop a background agent run by run_id",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "Run id returned by agent_run" }
            },
            "required": ["run_id"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &carve_state::Context<'_>,
    ) -> Result<ToolResult, crate::contracts::traits::tool::ToolError> {
        let tool_name = "agent_stop";
        let run_id = match required_string(&args, "run_id", tool_name) {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };
        let owner_thread_id = ctx
            .runtime_ref()
            .and_then(|rt| rt.value(RUNTIME_CALLER_SESSION_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(owner_thread_id) = owner_thread_id else {
            return Ok(tool_error(
                tool_name,
                "missing_runtime",
                "missing caller thread context",
            ));
        };

        let mut persisted_runs = parse_persisted_runs(ctx);
        let mut tree_ids = collect_descendant_run_ids_from_state(&persisted_runs, &run_id, true);
        if tree_ids.is_empty() {
            tree_ids.push(run_id.clone());
        }

        let mut summaries: HashMap<String, AgentRunSummary> = HashMap::new();
        let mut manager_error = None;

        match self
            .manager
            .stop_owned_tree(&owner_thread_id, &run_id)
            .await
        {
            Ok(stopped) => {
                for summary in stopped {
                    summaries.insert(summary.run_id.clone(), summary);
                }
            }
            Err(e) => {
                manager_error = Some(e);
            }
        }

        let mut stopped_any = !summaries.is_empty();
        for id in &tree_ids {
            let Some(run) = persisted_runs.get_mut(id) else {
                continue;
            };
            if run.status != AgentRunStatus::Running {
                continue;
            }

            if let Some(summary) = summaries.remove(id) {
                run.status = summary.status;
                run.assistant = summary.assistant;
                run.error = summary.error;
            } else {
                let stopped = make_orphaned_running_state(run);
                *run = stopped;
            }
            stopped_any = true;
            set_persisted_run(ctx, id, run.clone());
        }

        if !stopped_any {
            if let Some(err) = manager_error {
                return Ok(tool_error(tool_name, "invalid_state", err));
            }
            return Ok(tool_error(
                tool_name,
                "invalid_state",
                format!("Run '{run_id}' cannot be stopped"),
            ));
        }

        if let Some(summary) = {
            if let Some(summary) = summaries.remove(&run_id) {
                Some(summary)
            } else {
                persisted_runs
                    .get(&run_id)
                    .map(|run| as_agent_run_summary(&run_id, run))
            }
        } {
            return Ok(to_tool_result(tool_name, summary));
        }

        let fallback_target = persisted_runs.remove(&run_id);
        if let Some(run) = fallback_target {
            return Ok(to_tool_result(
                tool_name,
                as_agent_run_summary(&run_id, &run),
            ));
        }

        Ok(tool_error(
            tool_name,
            "invalid_state",
            "No matching run state for stopped run",
        ))
    }
}

#[derive(Clone)]
pub struct AgentRecoveryPlugin {
    manager: Arc<AgentRunManager>,
}

impl AgentRecoveryPlugin {
    pub fn new(manager: Arc<AgentRunManager>) -> Self {
        Self { manager }
    }

    async fn on_session_start(&self, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        let state = match step.thread.rebuild_state() {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut runs = parse_persisted_runs_from_doc(&state);
        if runs.is_empty() {
            return;
        }

        let has_pending_interaction = state
            .get(AGENT_STATE_PATH)
            .and_then(|a| a.get("pending_interaction"))
            .is_some_and(|v| !v.is_null());

        let outcome =
            reconcile_persisted_runs(self.manager.as_ref(), &step.thread.id, &mut runs).await;
        if outcome.changed {
            if let Some(patch) = set_agent_runs_patch_from_state_doc(
                &state,
                runs.clone(),
                &format!("agent_recovery_reconcile_{}", step.thread.id),
            ) {
                step.pending_patches.push(patch);
            }
        }

        if has_pending_interaction || outcome.orphaned_run_ids.is_empty() {
            return;
        }

        let run_id = outcome.orphaned_run_ids[0].clone();
        let Some(run) = runs.get(&run_id) else {
            return;
        };

        let behavior = ctx.get_permission(AGENT_RECOVERY_INTERACTION_ACTION);
        match behavior {
            ToolPermissionBehavior::Allow => {
                schedule_recovery_replay(&state, step, &run_id);
            }
            ToolPermissionBehavior::Deny => {}
            ToolPermissionBehavior::Ask => {
                let interaction = build_recovery_interaction(&run_id, run);
                if let Some(patch) =
                    set_pending_interaction_patch(&state, interaction, "agent_recovery_pending")
                {
                    step.pending_patches.push(patch);
                }
            }
        }
    }

    async fn on_before_inference(&self, step: &mut StepContext<'_>) {
        let state = match step.thread.rebuild_state() {
            Ok(v) => v,
            Err(_) => return,
        };

        let Some(pending) = parse_pending_interaction_from_state(&state) else {
            return;
        };
        if pending.action == AGENT_RECOVERY_INTERACTION_ACTION {
            step.skip_inference = true;
        }
    }
}

#[async_trait]
impl AgentPlugin for AgentRecoveryPlugin {
    fn id(&self) -> &str {
        "agent_recovery"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        match phase {
            Phase::SessionStart => self.on_session_start(step, ctx).await,
            Phase::BeforeInference => self.on_before_inference(step).await,
            _ => {}
        }
    }
}

#[derive(Clone)]
pub struct AgentToolsPlugin {
    agents: Arc<dyn AgentRegistry>,
    manager: Arc<AgentRunManager>,
    max_entries: usize,
    max_chars: usize,
}

impl AgentToolsPlugin {
    pub fn new(agents: Arc<dyn AgentRegistry>, manager: Arc<AgentRunManager>) -> Self {
        Self {
            agents,
            manager,
            max_entries: 64,
            max_chars: 16 * 1024,
        }
    }

    pub fn with_limits(mut self, max_entries: usize, max_chars: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self.max_chars = max_chars.max(256);
        self
    }

    fn render_available_agents(
        &self,
        caller_agent: Option<&str>,
        runtime: Option<&carve_state::Runtime>,
    ) -> String {
        let mut ids = self.agents.ids();
        ids.sort();
        if let Some(caller) = caller_agent {
            ids.retain(|id| id != caller);
        }
        ids.retain(|id| {
            is_runtime_allowed(
                runtime,
                id,
                RUNTIME_ALLOWED_AGENTS_KEY,
                RUNTIME_EXCLUDED_AGENTS_KEY,
            )
        });
        if ids.is_empty() {
            return String::new();
        }

        let total = ids.len();
        let mut out = String::new();
        out.push_str("<available_agents>\n");

        let mut shown = 0usize;
        for id in ids.into_iter().take(self.max_entries) {
            out.push_str("<agent>\n");
            out.push_str(&format!("<id>{}</id>\n", id));
            out.push_str("</agent>\n");
            shown += 1;
            if out.len() >= self.max_chars {
                break;
            }
        }

        out.push_str("</available_agents>\n");
        if shown < total {
            out.push_str(&format!(
                "Note: available_agents truncated (total={}, shown={}).\n",
                total, shown
            ));
        }

        out.push_str("<agent_tools_usage>\n");
        out.push_str("Run or resume: tool \"agent_run\" with {\"agent_id\":\"<id>\",\"prompt\":\"...\",\"fork_context\":false,\"background\":false}.\n");
        out.push_str("Resume existing run: tool \"agent_run\" with {\"run_id\":\"...\",\"prompt\":\"optional\",\"background\":false}.\n");
        out.push_str(
            "Stop running background run: tool \"agent_stop\" with {\"run_id\":\"...\"}.\n",
        );
        out.push_str("Statuses: running, completed, failed, stopped (stopped can be resumed).\n");
        out.push_str("</agent_tools_usage>");

        if out.len() > self.max_chars {
            out.truncate(self.max_chars);
        }

        out.trim_end().to_string()
    }

    async fn maybe_reminder(&self, step: &mut StepContext<'_>) {
        let owner_thread_id = step.thread.id.as_str();
        let runs = self
            .manager
            .running_or_stopped_for_owner(owner_thread_id)
            .await;
        if runs.is_empty() {
            return;
        }

        let mut s = String::new();
        s.push_str("<agent_runs>\n");
        let total = runs.len();
        let mut shown = 0usize;
        for r in runs.into_iter().take(self.max_entries) {
            s.push_str(&format!(
                "<run id=\"{}\" agent=\"{}\" status=\"{}\"/>\n",
                r.run_id,
                r.target_agent_id,
                r.status.as_str(),
            ));
            shown += 1;
            if s.len() >= self.max_chars {
                break;
            }
        }
        s.push_str("</agent_runs>\n");
        if shown < total {
            s.push_str(&format!(
                "Note: agent_runs truncated (total={}, shown={}).\n",
                total, shown
            ));
        }
        s.push_str("Use tool \"agent_run\" with run_id to resume/check, and \"agent_stop\" to stop running runs.");
        if s.len() > self.max_chars {
            s.truncate(self.max_chars);
        }
        step.reminder(s);
    }
}

#[async_trait]
impl AgentPlugin for AgentToolsPlugin {
    fn id(&self) -> &str {
        "agent_tools"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
        match phase {
            Phase::BeforeInference => {
                let caller_agent = step
                    .thread
                    .runtime
                    .value(RUNTIME_CALLER_AGENT_ID_KEY)
                    .and_then(|v| v.as_str());
                let rendered =
                    self.render_available_agents(caller_agent, Some(&step.thread.runtime));
                if !rendered.is_empty() {
                    step.system(rendered);
                }
            }
            Phase::AfterToolExecute => {
                // Inject system reminders after tool execution so the reminder is persisted
                // as internal-system history for subsequent turns.
                self.maybe_reminder(step).await;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::traits::tool::ToolStatus;
    use crate::orchestrator::InMemoryAgentRegistry;
    use crate::runtime::loop_runner::{
        TOOL_RUNTIME_CALLER_AGENT_ID_KEY, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
        TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
    };
    use crate::thread::Thread;
    use async_trait::async_trait;
    use carve_state::apply_patches;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn plugin_filters_out_caller_agent() {
        let mut reg = InMemoryAgentRegistry::new();
        reg.upsert(
            "a",
            crate::runtime::loop_runner::AgentDefinition::new("mock"),
        );
        reg.upsert(
            "b",
            crate::runtime::loop_runner::AgentDefinition::new("mock"),
        );
        let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(AgentRunManager::new()));
        let rendered = plugin.render_available_agents(Some("a"), None);
        assert!(rendered.contains("<id>b</id>"));
        assert!(!rendered.contains("<id>a</id>"));
    }

    #[test]
    fn plugin_filters_agents_by_runtime_policy() {
        let mut reg = InMemoryAgentRegistry::new();
        reg.upsert(
            "writer",
            crate::runtime::loop_runner::AgentDefinition::new("mock"),
        );
        reg.upsert(
            "reviewer",
            crate::runtime::loop_runner::AgentDefinition::new("mock"),
        );
        let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(AgentRunManager::new()));
        let mut rt = carve_state::Runtime::new();
        rt.set(RUNTIME_ALLOWED_AGENTS_KEY, vec!["writer"]).unwrap();
        let rendered = plugin.render_available_agents(None, Some(&rt));
        assert!(rendered.contains("<id>writer</id>"));
        assert!(!rendered.contains("<id>reviewer</id>"));
    }

    #[tokio::test]
    async fn plugin_adds_reminder_for_running_and_stopped_runs() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
        let mut reg = InMemoryAgentRegistry::new();
        reg.upsert(
            "worker",
            crate::runtime::loop_runner::AgentDefinition::new("mock"),
        );
        let manager = Arc::new(AgentRunManager::new());
        let plugin = AgentToolsPlugin::new(Arc::new(reg), manager.clone());

        let epoch = manager
            .put_running(
                "run-1",
                "owner-1".to_string(),
                "worker".to_string(),
                None,
                Thread::new("child-1"),
                None,
            )
            .await;
        assert_eq!(epoch, 1);

        let owner = Thread::new("owner-1");
        let mut step = StepContext::new(&owner, vec![]);
        plugin
            .on_phase(Phase::AfterToolExecute, &mut step, &ctx)
            .await;
        let reminder = step
            .system_reminders
            .first()
            .expect("running reminder should be present");
        assert!(reminder.contains("status=\"running\""));

        manager.stop_owned_tree("owner-1", "run-1").await.unwrap();
        let mut step2 = StepContext::new(&owner, vec![]);
        plugin
            .on_phase(Phase::AfterToolExecute, &mut step2, &ctx)
            .await;
        let reminder2 = step2
            .system_reminders
            .first()
            .expect("stopped reminder should be present");
        assert!(reminder2.contains("status=\"stopped\""));
    }

    #[tokio::test]
    async fn manager_ignores_stale_completion_by_epoch() {
        let manager = AgentRunManager::new();
        let epoch1 = manager
            .put_running(
                "run-1",
                "owner".to_string(),
                "agent-a".to_string(),
                None,
                Thread::new("s-1"),
                None,
            )
            .await;
        assert_eq!(epoch1, 1);

        let epoch2 = manager
            .put_running(
                "run-1",
                "owner".to_string(),
                "agent-a".to_string(),
                None,
                Thread::new("s-2"),
                None,
            )
            .await;
        assert_eq!(epoch2, 2);

        let ignored = manager
            .update_after_completion(
                "run-1",
                epoch1,
                AgentRunCompletion {
                    thread: Thread::new("old"),
                    status: AgentRunStatus::Completed,
                    assistant: Some("old".to_string()),
                    error: None,
                },
            )
            .await;
        assert!(ignored.is_none());

        let summary = manager
            .get_owned_summary("owner", "run-1")
            .await
            .expect("run should still exist");
        assert_eq!(summary.status, AgentRunStatus::Running);

        let applied = manager
            .update_after_completion(
                "run-1",
                epoch2,
                AgentRunCompletion {
                    thread: Thread::new("new"),
                    status: AgentRunStatus::Completed,
                    assistant: Some("new".to_string()),
                    error: None,
                },
            )
            .await
            .expect("latest epoch completion should apply");
        assert_eq!(applied.status, AgentRunStatus::Completed);
        assert_eq!(applied.assistant.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn agent_run_tool_requires_runtime_context() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini"),
            )
            .build()
            .unwrap();
        let tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));
        let doc = json!({});
        let ctx = carve_state::Context::new(&doc, "call-1", "tool:agent_run");
        let result = tool
            .execute(
                json!({"agent_id":"worker","prompt":"hi","background":false}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.status, ToolStatus::Error);
        assert!(result
            .message
            .unwrap_or_default()
            .contains("missing caller thread context"));
    }

    #[tokio::test]
    async fn agent_run_tool_rejects_disallowed_target_agent() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini"),
            )
            .with_agent(
                "reviewer",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini"),
            )
            .build()
            .unwrap();
        let tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));
        let doc = json!({});
        let mut rt = caller_runtime();
        rt.set(RUNTIME_ALLOWED_AGENTS_KEY, vec!["worker"]).unwrap();
        let ctx =
            carve_state::Context::new(&doc, "call-1", "tool:agent_run").with_runtime(Some(&rt));
        let result = tool
            .execute(
                json!({"agent_id":"reviewer","prompt":"hi","background":false}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.status, ToolStatus::Error);
        assert!(result
            .message
            .unwrap_or_default()
            .contains("Unknown or unavailable agent_id"));
    }

    #[derive(Debug)]
    struct SlowSkipPlugin;

    #[async_trait]
    impl AgentPlugin for SlowSkipPlugin {
        fn id(&self) -> &str {
            "slow_skip"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
            if phase == Phase::BeforeInference {
                tokio::time::sleep(Duration::from_millis(120)).await;
                step.skip_inference = true;
            }
        }
    }

    fn caller_runtime_with_state_and_run(
        state: serde_json::Value,
        run_id: &str,
    ) -> carve_state::Runtime {
        let mut rt = carve_state::Runtime::new();
        rt.set(TOOL_RUNTIME_CALLER_THREAD_ID_KEY, "owner-thread")
            .unwrap();
        rt.set(TOOL_RUNTIME_CALLER_AGENT_ID_KEY, "caller").unwrap();
        rt.set(RUNTIME_RUN_ID_KEY, run_id).unwrap();
        rt.set(TOOL_RUNTIME_CALLER_STATE_KEY, state).unwrap();
        rt.set(
            TOOL_RUNTIME_CALLER_MESSAGES_KEY,
            vec![crate::types::Message::user("seed message")],
        )
        .unwrap();
        rt
    }

    fn caller_runtime_with_state(state: serde_json::Value) -> carve_state::Runtime {
        caller_runtime_with_state_and_run(state, "parent-run-default")
    }

    fn caller_runtime() -> carve_state::Runtime {
        caller_runtime_with_state(json!({"forked": true}))
    }

    #[tokio::test]
    async fn background_stop_then_resume_completes() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                    .with_plugin(Arc::new(SlowSkipPlugin)),
            )
            .build()
            .unwrap();
        let manager = Arc::new(AgentRunManager::new());
        let run_tool = AgentRunTool::new(os, manager.clone());
        let stop_tool = AgentStopTool::new(manager);

        let doc = json!({});
        let rt = caller_runtime();
        let ctx =
            carve_state::Context::new(&doc, "call-run", "tool:agent_run").with_runtime(Some(&rt));
        let started = run_tool
            .execute(
                json!({
                    "agent_id":"worker",
                    "prompt":"start",
                    "background": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(started.status, ToolStatus::Success);
        assert_eq!(started.data["status"], json!("running"));
        let run_id = started.data["run_id"]
            .as_str()
            .expect("run_id should exist")
            .to_string();

        let stop_ctx =
            carve_state::Context::new(&doc, "call-stop", "tool:agent_stop").with_runtime(Some(&rt));
        let stopped = stop_tool
            .execute(json!({ "run_id": run_id.clone() }), &stop_ctx)
            .await
            .unwrap();
        assert_eq!(stopped.status, ToolStatus::Success);
        assert_eq!(stopped.data["status"], json!("stopped"));

        // Give cancelled background task a chance to flush stale completion.
        tokio::time::sleep(Duration::from_millis(30)).await;

        let resumed = run_tool
            .execute(
                json!({
                    "run_id": run_id,
                    "prompt":"resume",
                    "background": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(resumed.status, ToolStatus::Success);
        assert_eq!(resumed.data["status"], json!("completed"));
    }

    #[tokio::test]
    async fn manager_stop_tree_stops_descendants() {
        let manager = AgentRunManager::new();
        manager
            .put_running(
                "parent-run",
                "owner-thread".to_string(),
                "agent-a".to_string(),
                None,
                Thread::new("parent-run-thread"),
                None,
            )
            .await;
        manager
            .put_running(
                "child-run",
                "owner-thread".to_string(),
                "agent-a".to_string(),
                Some("parent-run".to_string()),
                Thread::new("child-run-thread"),
                None,
            )
            .await;
        manager
            .put_running(
                "grandchild-run",
                "owner-thread".to_string(),
                "agent-a".to_string(),
                Some("child-run".to_string()),
                Thread::new("grandchild-run-thread"),
                None,
            )
            .await;
        manager
            .put_running(
                "other-owner-run",
                "other-owner".to_string(),
                "agent-b".to_string(),
                Some("parent-run".to_string()),
                Thread::new("other-owner-thread"),
                None,
            )
            .await;

        let stopped = manager
            .stop_owned_tree("owner-thread", "parent-run")
            .await
            .unwrap();

        assert_eq!(stopped.len(), 3);

        let parent = manager
            .get_owned_summary("owner-thread", "parent-run")
            .await
            .expect("parent run should exist");
        assert_eq!(parent.status, AgentRunStatus::Stopped);

        let child = manager
            .get_owned_summary("owner-thread", "child-run")
            .await
            .expect("child run should exist");
        assert_eq!(child.status, AgentRunStatus::Stopped);

        let grandchild = manager
            .get_owned_summary("owner-thread", "grandchild-run")
            .await
            .expect("grandchild run should exist");
        assert_eq!(grandchild.status, AgentRunStatus::Stopped);

        let denied = manager
            .stop_owned_tree("owner-thread", "other-owner-run")
            .await;
        assert!(denied.is_err());
    }

    #[tokio::test]
    async fn agent_run_tool_persists_run_state_patch() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                    .with_plugin(Arc::new(SlowSkipPlugin)),
            )
            .build()
            .unwrap();
        let run_tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));

        let doc = json!({});
        let rt = caller_runtime();
        let ctx =
            carve_state::Context::new(&doc, "call-run", "tool:agent_run").with_runtime(Some(&rt));
        let started = run_tool
            .execute(
                json!({
                    "agent_id":"worker",
                    "prompt":"start",
                    "background": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(started.status, ToolStatus::Success);
        let run_id = started.data["run_id"]
            .as_str()
            .expect("run_id should exist")
            .to_string();

        let patch = ctx.take_patch();
        assert!(
            !patch.patch().is_empty(),
            "expected tool to persist run snapshot into state"
        );
        let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
        assert_eq!(
            updated["agent"]["agent_runs"][&run_id]["status"],
            json!("running")
        );
    }

    #[tokio::test]
    async fn agent_run_tool_binds_runtime_run_id_and_parent_lineage() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                    .with_plugin(Arc::new(SlowSkipPlugin)),
            )
            .build()
            .unwrap();
        let manager = Arc::new(AgentRunManager::new());
        let run_tool = AgentRunTool::new(os, manager.clone());

        let doc = json!({});
        let rt = caller_runtime_with_state_and_run(json!({"forked": true}), "parent-run-42");
        let ctx =
            carve_state::Context::new(&doc, "call-run", "tool:agent_run").with_runtime(Some(&rt));
        let started = run_tool
            .execute(
                json!({
                    "agent_id":"worker",
                    "prompt":"start",
                    "background": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(started.status, ToolStatus::Success);
        let run_id = started.data["run_id"]
            .as_str()
            .expect("run_id should exist")
            .to_string();

        let child_thread = manager
            .owned_record("owner-thread", &run_id)
            .await
            .expect("child thread should be tracked");
        assert_eq!(
            child_thread
                .runtime
                .value(RUNTIME_RUN_ID_KEY)
                .and_then(|v| v.as_str()),
            Some(run_id.as_str())
        );
        assert_eq!(
            child_thread
                .runtime
                .value(RUNTIME_PARENT_RUN_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("parent-run-42")
        );

        let patch = ctx.take_patch();
        let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
        assert_eq!(
            updated["agent"]["agent_runs"][&run_id]["parent_run_id"],
            json!("parent-run-42")
        );
    }

    #[tokio::test]
    async fn agent_run_tool_resumes_from_persisted_state_without_live_record() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                    .with_plugin(Arc::new(SlowSkipPlugin)),
            )
            .build()
            .unwrap();
        let run_tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));

        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let doc = json!({
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "stopped",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        });
        let rt = caller_runtime_with_state(doc.clone());
        let ctx =
            carve_state::Context::new(&doc, "call-run", "tool:agent_run").with_runtime(Some(&rt));
        let resumed = run_tool
            .execute(
                json!({
                    "run_id":"run-1",
                    "prompt":"resume",
                    "background": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(resumed.status, ToolStatus::Success);
        assert_eq!(resumed.data["status"], json!("completed"));
    }

    #[tokio::test]
    async fn agent_run_tool_resume_updates_parent_run_lineage() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                    .with_plugin(Arc::new(SlowSkipPlugin)),
            )
            .build()
            .unwrap();
        let manager = Arc::new(AgentRunManager::new());
        let run_tool = AgentRunTool::new(os, manager.clone());

        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let doc = json!({
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "parent_run_id": "old-parent",
                        "target_agent_id": "worker",
                        "status": "stopped",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        });
        let rt = caller_runtime_with_state_and_run(doc.clone(), "new-parent-run");
        let ctx =
            carve_state::Context::new(&doc, "call-run", "tool:agent_run").with_runtime(Some(&rt));
        let resumed = run_tool
            .execute(
                json!({
                    "run_id":"run-1",
                    "prompt":"resume",
                    "background": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(resumed.status, ToolStatus::Success);

        let child_thread = manager
            .owned_record("owner-thread", "run-1")
            .await
            .expect("resumed run should be tracked");
        assert_eq!(
            child_thread
                .runtime
                .value(RUNTIME_RUN_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("run-1")
        );
        assert_eq!(
            child_thread
                .runtime
                .value(RUNTIME_PARENT_RUN_ID_KEY)
                .and_then(|v| v.as_str()),
            Some("new-parent-run")
        );

        let patch = ctx.take_patch();
        let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
        assert_eq!(
            updated["agent"]["agent_runs"]["run-1"]["parent_run_id"],
            json!("new-parent-run")
        );
    }

    #[tokio::test]
    async fn agent_run_tool_marks_orphan_running_as_stopped_before_resume() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::runtime::loop_runner::AgentDefinition::new("gpt-4o-mini")
                    .with_plugin(Arc::new(SlowSkipPlugin)),
            )
            .build()
            .unwrap();
        let run_tool = AgentRunTool::new(os, Arc::new(AgentRunManager::new()));

        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let doc = json!({
            "agent": {
                "agent_runs": {
                    "run-1": {
                        "run_id": "run-1",
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(&child_thread).unwrap()
                    }
                }
            }
        });
        let rt = caller_runtime_with_state(doc.clone());
        let ctx =
            carve_state::Context::new(&doc, "call-run", "tool:agent_run").with_runtime(Some(&rt));
        let summary = run_tool
            .execute(
                json!({
                    "run_id":"run-1",
                    "background": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(summary.status, ToolStatus::Success);
        assert_eq!(summary.data["status"], json!("stopped"));
    }

    #[tokio::test]
    async fn agent_stop_tool_stops_descendant_runs() {
        let manager = Arc::new(AgentRunManager::new());
        let stop_tool = AgentStopTool::new(manager.clone());
        let os_thread = Thread::new("owner-thread");

        let parent_thread = Thread::new("parent-s");
        let child_thread = Thread::new("child-s");
        let grandchild_thread = Thread::new("grandchild-s");
        let parent_run_id = "run-parent";
        let child_run_id = "run-child";
        let grandchild_run_id = "run-grandchild";

        manager
            .put_running(
                parent_run_id,
                os_thread.id.clone(),
                "worker".to_string(),
                None,
                parent_thread.clone(),
                None,
            )
            .await;
        manager
            .put_running(
                child_run_id,
                os_thread.id.clone(),
                "worker".to_string(),
                Some(parent_run_id.to_string()),
                child_thread.clone(),
                None,
            )
            .await;
        manager
            .put_running(
                grandchild_run_id,
                os_thread.id.clone(),
                "worker".to_string(),
                Some(child_run_id.to_string()),
                grandchild_thread.clone(),
                None,
            )
            .await;

        let doc = json!({
            "agent": {
                "agent_runs": {
                    parent_run_id: {
                        "run_id": parent_run_id,
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(parent_thread).unwrap()
                    },
                    child_run_id: {
                        "run_id": child_run_id,
                        "parent_run_id": parent_run_id,
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(child_thread).unwrap()
                    },
                    grandchild_run_id: {
                        "run_id": grandchild_run_id,
                        "parent_run_id": child_run_id,
                        "target_agent_id": "worker",
                        "status": "running",
                        "thread": serde_json::to_value(grandchild_thread).unwrap()
                    }
                }
            }
        });

        let mut rt = carve_state::Runtime::new();
        rt.set(TOOL_RUNTIME_CALLER_THREAD_ID_KEY, os_thread.id.clone())
            .unwrap();
        let ctx =
            carve_state::Context::new(&doc, "call-stop", "tool:agent_stop").with_runtime(Some(&rt));
        let result = stop_tool
            .execute(json!({ "run_id": parent_run_id }), &ctx)
            .await
            .unwrap();
        assert_eq!(result.status, ToolStatus::Success);
        assert_eq!(result.data["status"], json!("stopped"));

        let parent = manager
            .get_owned_summary(&os_thread.id, parent_run_id)
            .await
            .expect("parent run should exist");
        assert_eq!(parent.status, AgentRunStatus::Stopped);
        let child = manager
            .get_owned_summary(&os_thread.id, child_run_id)
            .await
            .expect("child run should exist");
        assert_eq!(child.status, AgentRunStatus::Stopped);
        let grandchild = manager
            .get_owned_summary(&os_thread.id, grandchild_run_id)
            .await
            .expect("grandchild run should exist");
        assert_eq!(grandchild.status, AgentRunStatus::Stopped);

        let patch = ctx.take_patch();
        let updated = apply_patches(&doc, std::iter::once(patch.patch())).unwrap();
        assert_eq!(
            updated["agent"]["agent_runs"][parent_run_id]["status"],
            json!("stopped")
        );
        assert_eq!(
            updated["agent"]["agent_runs"][child_run_id]["status"],
            json!("stopped")
        );
        assert_eq!(
            updated["agent"]["agent_runs"][grandchild_run_id]["status"],
            json!("stopped")
        );
    }

    #[tokio::test]
    async fn recovery_plugin_reconciles_orphan_running_and_requests_confirmation() {
        let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let thread = Thread::with_initial_state(
            "owner-1",
            json!({
                "agent": {
                    "agent_runs": {
                        "run-1": {
                            "run_id": "run-1",
                            "target_agent_id": "worker",
                            "status": "running",
                            "thread": serde_json::to_value(&child_thread).unwrap()
                        }
                    }
                }
            }),
        );
        let doc = thread.rebuild_state().unwrap();
        let ctx = Context::new(&doc, "test", "test");
        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;
        assert!(
            !step.pending_patches.is_empty(),
            "expected reconciliation + pending patches for orphan running entry"
        );
        assert!(!step.skip_inference);

        let updated = thread
            .clone()
            .with_patches(step.pending_patches.clone())
            .rebuild_state()
            .unwrap();
        assert_eq!(
            updated["agent"]["agent_runs"]["run-1"]["status"],
            json!("stopped")
        );
        assert_eq!(
            updated["agent"]["pending_interaction"]["action"],
            json!(AGENT_RECOVERY_INTERACTION_ACTION)
        );
        assert_eq!(
            updated["agent"]["pending_interaction"]["parameters"]["run_id"],
            json!("run-1")
        );

        let updated_thread = thread.clone().with_patches(step.pending_patches);
        let mut before = StepContext::new(&updated_thread, vec![]);
        plugin
            .on_phase(Phase::BeforeInference, &mut before, &ctx)
            .await;
        assert!(
            before.skip_inference,
            "recovery confirmation should pause inference"
        );
    }

    #[tokio::test]
    async fn recovery_plugin_does_not_override_existing_pending_interaction() {
        let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let thread = Thread::with_initial_state(
            "owner-1",
            json!({
                "agent": {
                    "pending_interaction": {
                        "id": "existing_1",
                        "action": "confirm",
                    },
                    "agent_runs": {
                        "run-1": {
                            "run_id": "run-1",
                            "target_agent_id": "worker",
                            "status": "running",
                            "thread": serde_json::to_value(&child_thread).unwrap()
                        }
                    }
                }
            }),
        );

        let doc = thread.rebuild_state().unwrap();
        let ctx = Context::new(&doc, "test", "test");
        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;
        assert!(
            !step.skip_inference,
            "existing pending interaction should not be replaced"
        );

        let updated = thread
            .clone()
            .with_patches(step.pending_patches)
            .rebuild_state()
            .unwrap();
        assert_eq!(
            updated["agent"]["pending_interaction"]["id"],
            json!("existing_1")
        );
    }

    #[tokio::test]
    async fn recovery_plugin_auto_approve_when_permission_allow() {
        let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let thread = Thread::with_initial_state(
            "owner-1",
            json!({
                "permissions": {
                    "default_behavior": "ask",
                    "tools": {
                        "recover_agent_run": "allow"
                    }
                },
                "agent": {
                    "agent_runs": {
                        "run-1": {
                            "run_id": "run-1",
                            "target_agent_id": "worker",
                            "status": "running",
                            "thread": serde_json::to_value(&child_thread).unwrap()
                        }
                    }
                }
            }),
        );
        let doc = thread.rebuild_state().unwrap();
        let ctx = Context::new(&doc, "test", "test");
        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let updated = thread
            .clone()
            .with_patches(step.pending_patches)
            .rebuild_state()
            .unwrap();
        let replay_calls: Vec<ToolCall> = updated["agent"]
            .get("replay_tool_calls")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].name, "agent_run");
        assert_eq!(replay_calls[0].arguments["run_id"], "run-1");
        assert_eq!(
            updated["agent"]["agent_runs"]["run-1"]["status"],
            json!("stopped")
        );
        assert!(
            updated["agent"].get("pending_interaction").is_none()
                || updated["agent"]["pending_interaction"].is_null()
        );
    }

    #[tokio::test]
    async fn recovery_plugin_auto_deny_when_permission_deny() {
        let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let thread = Thread::with_initial_state(
            "owner-1",
            json!({
                "permissions": {
                    "default_behavior": "ask",
                    "tools": {
                        "recover_agent_run": "deny"
                    }
                },
                "agent": {
                    "agent_runs": {
                        "run-1": {
                            "run_id": "run-1",
                            "target_agent_id": "worker",
                            "status": "running",
                            "thread": serde_json::to_value(&child_thread).unwrap()
                        }
                    }
                }
            }),
        );
        let doc = thread.rebuild_state().unwrap();
        let ctx = Context::new(&doc, "test", "test");
        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let updated = thread
            .clone()
            .with_patches(step.pending_patches)
            .rebuild_state()
            .unwrap();
        let replay_calls: Vec<ToolCall> = updated["agent"]
            .get("replay_tool_calls")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        assert!(replay_calls.is_empty());
        assert_eq!(
            updated["agent"]["agent_runs"]["run-1"]["status"],
            json!("stopped")
        );
        assert!(
            updated["agent"].get("pending_interaction").is_none()
                || updated["agent"]["pending_interaction"].is_null()
        );
    }

    #[tokio::test]
    async fn recovery_plugin_auto_approve_from_default_behavior_allow() {
        let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let thread = Thread::with_initial_state(
            "owner-1",
            json!({
                "permissions": {
                    "default_behavior": "allow",
                    "tools": {}
                },
                "agent": {
                    "agent_runs": {
                        "run-1": {
                            "run_id": "run-1",
                            "target_agent_id": "worker",
                            "status": "running",
                            "thread": serde_json::to_value(&child_thread).unwrap()
                        }
                    }
                }
            }),
        );
        let doc = thread.rebuild_state().unwrap();
        let ctx = Context::new(&doc, "test", "test");
        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let updated = thread
            .clone()
            .with_patches(step.pending_patches)
            .rebuild_state()
            .unwrap();
        let replay_calls: Vec<ToolCall> = updated["agent"]
            .get("replay_tool_calls")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].name, "agent_run");
        assert_eq!(replay_calls[0].arguments["run_id"], "run-1");
        assert!(
            updated["agent"].get("pending_interaction").is_none()
                || updated["agent"]["pending_interaction"].is_null()
        );
    }

    #[tokio::test]
    async fn recovery_plugin_auto_deny_from_default_behavior_deny() {
        let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let thread = Thread::with_initial_state(
            "owner-1",
            json!({
                "permissions": {
                    "default_behavior": "deny",
                    "tools": {}
                },
                "agent": {
                    "agent_runs": {
                        "run-1": {
                            "run_id": "run-1",
                            "target_agent_id": "worker",
                            "status": "running",
                            "thread": serde_json::to_value(&child_thread).unwrap()
                        }
                    }
                }
            }),
        );
        let doc = thread.rebuild_state().unwrap();
        let ctx = Context::new(&doc, "test", "test");
        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let updated = thread
            .clone()
            .with_patches(step.pending_patches)
            .rebuild_state()
            .unwrap();
        let replay_calls: Vec<ToolCall> = updated["agent"]
            .get("replay_tool_calls")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        assert!(
            replay_calls.is_empty(),
            "deny should not schedule recovery replay"
        );
        assert!(
            updated["agent"].get("pending_interaction").is_none()
                || updated["agent"]["pending_interaction"].is_null()
        );
    }

    #[tokio::test]
    async fn recovery_plugin_tool_rule_overrides_default_behavior() {
        let plugin = AgentRecoveryPlugin::new(Arc::new(AgentRunManager::new()));
        let child_thread = crate::thread::Thread::new("child-run")
            .with_message(crate::types::Message::user("seed"));
        let thread = Thread::with_initial_state(
            "owner-1",
            json!({
                "permissions": {
                    "default_behavior": "allow",
                    "tools": {
                        "recover_agent_run": "ask"
                    }
                },
                "agent": {
                    "agent_runs": {
                        "run-1": {
                            "run_id": "run-1",
                            "target_agent_id": "worker",
                            "status": "running",
                            "thread": serde_json::to_value(&child_thread).unwrap()
                        }
                    }
                }
            }),
        );
        let doc = thread.rebuild_state().unwrap();
        let ctx = Context::new(&doc, "test", "test");
        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let updated = thread
            .clone()
            .with_patches(step.pending_patches)
            .rebuild_state()
            .unwrap();
        let replay_calls: Vec<ToolCall> = updated["agent"]
            .get("replay_tool_calls")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        assert!(
            replay_calls.is_empty(),
            "tool-level ask should override default allow"
        );
        assert_eq!(
            updated["agent"]["pending_interaction"]["action"],
            json!(AGENT_RECOVERY_INTERACTION_ACTION)
        );
    }
}
