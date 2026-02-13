use super::{AgentOs, AgentRegistry};
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
pub(crate) use crate::r#loop::TOOL_RUNTIME_CALLER_AGENT_ID_KEY as RUNTIME_CALLER_AGENT_ID_KEY;
use crate::r#loop::{
    RunContext, TOOL_RUNTIME_CALLER_MESSAGES_KEY, TOOL_RUNTIME_CALLER_SESSION_ID_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY,
};
use crate::stop::StopReason;
use crate::tool_filter::{
    is_runtime_allowed, RUNTIME_ALLOWED_AGENTS_KEY, RUNTIME_EXCLUDED_AGENTS_KEY,
};
use crate::traits::tool::{Tool, ToolDescriptor, ToolResult, ToolStatus};
use crate::types::{Message, Role};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const RUNTIME_CALLER_SESSION_ID_KEY: &str = TOOL_RUNTIME_CALLER_SESSION_ID_KEY;
const RUNTIME_CALLER_STATE_KEY: &str = TOOL_RUNTIME_CALLER_STATE_KEY;
const RUNTIME_CALLER_MESSAGES_KEY: &str = TOOL_RUNTIME_CALLER_MESSAGES_KEY;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRunStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl AgentRunStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

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
    owner_session_id: String,
    target_agent_id: String,
    status: AgentRunStatus,
    session: crate::Session,
    assistant: Option<String>,
    error: Option<String>,
    stop_requested: bool,
    cancellation_token: Option<CancellationToken>,
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
        owner_session_id: &str,
        run_id: &str,
    ) -> Option<AgentRunSummary> {
        let runs = self.runs.lock().await;
        let rec = runs.get(run_id)?;
        if rec.owner_session_id != owner_session_id {
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
        owner_session_id: &str,
    ) -> Vec<AgentRunSummary> {
        let runs = self.runs.lock().await;
        let mut out: Vec<AgentRunSummary> = runs
            .iter()
            .filter_map(|(run_id, rec)| {
                if rec.owner_session_id != owner_session_id {
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

    pub async fn stop_owned(
        &self,
        owner_session_id: &str,
        run_id: &str,
    ) -> Result<AgentRunSummary, String> {
        let mut runs = self.runs.lock().await;
        let Some(rec) = runs.get_mut(run_id) else {
            return Err(format!("Unknown run_id: {run_id}"));
        };
        if rec.owner_session_id != owner_session_id {
            return Err(format!("Unknown run_id: {run_id}"));
        }
        if rec.status != AgentRunStatus::Running {
            return Err(format!(
                "Run '{run_id}' is not running (current status: {})",
                rec.status.as_str()
            ));
        }
        rec.stop_requested = true;
        rec.status = AgentRunStatus::Stopped;
        if let Some(token) = rec.cancellation_token.take() {
            token.cancel();
        }
        Ok(AgentRunSummary {
            run_id: run_id.to_string(),
            target_agent_id: rec.target_agent_id.clone(),
            status: rec.status,
            assistant: rec.assistant.clone(),
            error: rec.error.clone(),
        })
    }

    async fn put_running(
        &self,
        run_id: &str,
        owner_session_id: String,
        target_agent_id: String,
        session: crate::Session,
        cancellation_token: Option<CancellationToken>,
    ) -> u64 {
        let mut runs = self.runs.lock().await;
        let epoch = runs.get(run_id).map(|r| r.epoch + 1).unwrap_or(1);
        runs.insert(
            run_id.to_string(),
            AgentRunRecord {
                epoch,
                owner_session_id,
                target_agent_id,
                status: AgentRunStatus::Running,
                session,
                assistant: None,
                error: None,
                stop_requested: false,
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
        rec.session = completion.session;
        rec.assistant = completion.assistant;
        rec.error = completion.error;

        // Explicit stop request wins over terminal status from executor.
        rec.status = if rec.stop_requested {
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
        owner_session_id: &str,
        run_id: &str,
    ) -> Result<AgentRunRecord, String> {
        let runs = self.runs.lock().await;
        let Some(rec) = runs.get(run_id) else {
            return Err(format!("Unknown run_id: {run_id}"));
        };
        if rec.owner_session_id != owner_session_id {
            return Err(format!("Unknown run_id: {run_id}"));
        }
        Ok(rec.clone())
    }
}

#[derive(Debug)]
struct AgentRunCompletion {
    session: crate::Session,
    status: AgentRunStatus,
    assistant: Option<String>,
    error: Option<String>,
}

fn last_assistant_message(session: &crate::Session) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.content.clone())
}

async fn execute_target_agent(
    os: AgentOs,
    target_agent_id: String,
    session: crate::Session,
    cancellation_token: Option<CancellationToken>,
) -> AgentRunCompletion {
    let run_ctx = RunContext { cancellation_token };
    let stream = match os.run_stream_with_session(&target_agent_id, session.clone(), run_ctx) {
        Ok(stream) => stream,
        Err(e) => {
            return AgentRunCompletion {
                session,
                status: AgentRunStatus::Failed,
                assistant: None,
                error: Some(e.to_string()),
            };
        }
    };

    let mut saw_error: Option<String> = None;
    let mut stop_reason: Option<StopReason> = None;
    let mut events = stream.events;

    while let Some(ev) = events.next().await {
        match ev {
            crate::AgentEvent::Error { message } => {
                if saw_error.is_none() {
                    saw_error = Some(message);
                }
            }
            crate::AgentEvent::RunFinish {
                stop_reason: reason,
                ..
            } => {
                stop_reason = reason;
            }
            _ => {}
        }
    }

    let final_session = stream.final_session.await.unwrap_or(session);
    let assistant = last_assistant_message(&final_session);

    if saw_error.is_some() {
        return AgentRunCompletion {
            session: final_session,
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
        session: final_session,
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
        .filter(|m| m.visibility == crate::Visibility::All)
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
    ) -> Result<ToolResult, crate::ToolError> {
        let tool_name = "agent_run";
        let run_id = optional_string(&args, "run_id");
        let background = required_bool(&args, "background", false);
        let fork_context = required_bool(&args, "fork_context", false);

        let runtime = ctx.runtime_ref();
        let owner_session_id = runtime
            .and_then(|rt| rt.value(RUNTIME_CALLER_SESSION_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(owner_session_id) = owner_session_id else {
            return Ok(tool_error(
                tool_name,
                "missing_runtime",
                "missing caller session context",
            ));
        };
        let caller_agent_id = runtime
            .and_then(|rt| rt.value(RUNTIME_CALLER_AGENT_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        if let Some(run_id) = run_id {
            if let Some(existing) = self
                .manager
                .get_owned_summary(&owner_session_id, &run_id)
                .await
            {
                match existing.status {
                    AgentRunStatus::Running
                    | AgentRunStatus::Completed
                    | AgentRunStatus::Failed => {
                        return Ok(to_tool_result(tool_name, existing));
                    }
                    AgentRunStatus::Stopped => {
                        let mut record = match self
                            .manager
                            .record_for_resume(&owner_session_id, &run_id)
                            .await
                        {
                            Ok(v) => v,
                            Err(e) => return Ok(tool_error(tool_name, "unknown_run", e)),
                        };

                        if let Some(prompt) = optional_string(&args, "prompt") {
                            record.session = record.session.with_message(Message::user(prompt));
                        }

                        if background {
                            let token = CancellationToken::new();
                            let epoch = self
                                .manager
                                .put_running(
                                    &run_id,
                                    owner_session_id.clone(),
                                    record.target_agent_id.clone(),
                                    record.session.clone(),
                                    Some(token.clone()),
                                )
                                .await;
                            let manager = self.manager.clone();
                            let os = self.os.clone();
                            let run_id_bg = run_id.clone();
                            let agent_id_bg = record.target_agent_id.clone();
                            let session_bg = record.session;
                            tokio::spawn(async move {
                                let completion =
                                    execute_target_agent(os, agent_id_bg, session_bg, Some(token))
                                        .await;
                                let _ = manager
                                    .update_after_completion(&run_id_bg, epoch, completion)
                                    .await;
                            });

                            let summary = self
                                .manager
                                .get_owned_summary(&owner_session_id, &run_id)
                                .await
                                .expect("summary must exist right after put_running");
                            return Ok(to_tool_result(tool_name, summary));
                        }

                        let epoch = self
                            .manager
                            .put_running(
                                &run_id,
                                owner_session_id.clone(),
                                record.target_agent_id.clone(),
                                record.session.clone(),
                                None,
                            )
                            .await;
                        let completion = execute_target_agent(
                            self.os.clone(),
                            record.target_agent_id,
                            record.session,
                            None,
                        )
                        .await;
                        let summary = self
                            .manager
                            .update_after_completion(&run_id, epoch, completion)
                            .await
                            .expect("summary must exist after completion update");
                        return Ok(to_tool_result(tool_name, summary));
                    }
                }
            }

            return Ok(tool_error(
                tool_name,
                "unknown_run",
                format!("Unknown run_id: {run_id}"),
            ));
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
        let session_id = format!("agent-run-{run_id}");

        let mut child_session = if fork_context {
            let fork_state = runtime
                .and_then(|rt| rt.value(RUNTIME_CALLER_STATE_KEY))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut forked = crate::Session::with_initial_state(session_id, fork_state);
            if let Some(messages) = parse_caller_messages(runtime) {
                forked = forked.with_messages(filtered_fork_messages(messages));
            }
            forked
        } else {
            crate::Session::new(session_id)
        };
        child_session = child_session.with_message(Message::user(prompt));

        if background {
            let token = CancellationToken::new();
            let epoch = self
                .manager
                .put_running(
                    &run_id,
                    owner_session_id.clone(),
                    target_agent_id.clone(),
                    child_session.clone(),
                    Some(token.clone()),
                )
                .await;
            let manager = self.manager.clone();
            let os = self.os.clone();
            let run_id_bg = run_id.clone();
            tokio::spawn(async move {
                let completion =
                    execute_target_agent(os, target_agent_id, child_session, Some(token)).await;
                let _ = manager
                    .update_after_completion(&run_id_bg, epoch, completion)
                    .await;
            });

            let summary = self
                .manager
                .get_owned_summary(&owner_session_id, &run_id)
                .await
                .expect("summary must exist right after put_running");
            return Ok(to_tool_result(tool_name, summary));
        }

        let epoch = self
            .manager
            .put_running(
                &run_id,
                owner_session_id.clone(),
                target_agent_id.clone(),
                child_session.clone(),
                None,
            )
            .await;
        let completion =
            execute_target_agent(self.os.clone(), target_agent_id, child_session, None).await;
        let summary = self
            .manager
            .update_after_completion(&run_id, epoch, completion)
            .await
            .expect("summary must exist after completion update");
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
    ) -> Result<ToolResult, crate::ToolError> {
        let tool_name = "agent_stop";
        let run_id = match required_string(&args, "run_id", tool_name) {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };
        let owner_session_id = ctx
            .runtime_ref()
            .and_then(|rt| rt.value(RUNTIME_CALLER_SESSION_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(owner_session_id) = owner_session_id else {
            return Ok(tool_error(
                tool_name,
                "missing_runtime",
                "missing caller session context",
            ));
        };

        match self.manager.stop_owned(&owner_session_id, &run_id).await {
            Ok(summary) => Ok(to_tool_result(tool_name, summary)),
            Err(e) => Ok(tool_error(tool_name, "invalid_state", e)),
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
        let owner_session_id = step.session.id.as_str();
        let runs = self
            .manager
            .running_or_stopped_for_owner(owner_session_id)
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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        match phase {
            Phase::BeforeInference => {
                let caller_agent = step
                    .session
                    .runtime
                    .value(RUNTIME_CALLER_AGENT_ID_KEY)
                    .and_then(|v| v.as_str());
                let rendered =
                    self.render_available_agents(caller_agent, Some(&step.session.runtime));
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

    fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_os::InMemoryAgentRegistry;
    use crate::r#loop::{
        TOOL_RUNTIME_CALLER_AGENT_ID_KEY, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
        TOOL_RUNTIME_CALLER_SESSION_ID_KEY, TOOL_RUNTIME_CALLER_STATE_KEY,
    };
    use crate::session::Session;
    use crate::traits::tool::ToolStatus;
    use async_trait::async_trait;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn plugin_filters_out_caller_agent() {
        let mut reg = InMemoryAgentRegistry::new();
        reg.upsert("a", crate::AgentDefinition::new("mock"));
        reg.upsert("b", crate::AgentDefinition::new("mock"));
        let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(AgentRunManager::new()));
        let rendered = plugin.render_available_agents(Some("a"), None);
        assert!(rendered.contains("<id>b</id>"));
        assert!(!rendered.contains("<id>a</id>"));
    }

    #[test]
    fn plugin_filters_agents_by_runtime_policy() {
        let mut reg = InMemoryAgentRegistry::new();
        reg.upsert("writer", crate::AgentDefinition::new("mock"));
        reg.upsert("reviewer", crate::AgentDefinition::new("mock"));
        let plugin = AgentToolsPlugin::new(Arc::new(reg), Arc::new(AgentRunManager::new()));
        let mut rt = carve_state::Runtime::new();
        rt.set(RUNTIME_ALLOWED_AGENTS_KEY, vec!["writer"]).unwrap();
        let rendered = plugin.render_available_agents(None, Some(&rt));
        assert!(rendered.contains("<id>writer</id>"));
        assert!(!rendered.contains("<id>reviewer</id>"));
    }

    #[tokio::test]
    async fn plugin_adds_reminder_for_running_and_stopped_runs() {
        let mut reg = InMemoryAgentRegistry::new();
        reg.upsert("worker", crate::AgentDefinition::new("mock"));
        let manager = Arc::new(AgentRunManager::new());
        let plugin = AgentToolsPlugin::new(Arc::new(reg), manager.clone());

        let epoch = manager
            .put_running(
                "run-1",
                "owner-1".to_string(),
                "worker".to_string(),
                Session::new("child-1"),
                None,
            )
            .await;
        assert_eq!(epoch, 1);

        let owner = Session::new("owner-1");
        let mut step = StepContext::new(&owner, vec![]);
        plugin.on_phase(Phase::AfterToolExecute, &mut step).await;
        let reminder = step
            .system_reminders
            .first()
            .expect("running reminder should be present");
        assert!(reminder.contains("status=\"running\""));

        manager.stop_owned("owner-1", "run-1").await.unwrap();
        let mut step2 = StepContext::new(&owner, vec![]);
        plugin.on_phase(Phase::AfterToolExecute, &mut step2).await;
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
                Session::new("s-1"),
                None,
            )
            .await;
        assert_eq!(epoch1, 1);

        let epoch2 = manager
            .put_running(
                "run-1",
                "owner".to_string(),
                "agent-a".to_string(),
                Session::new("s-2"),
                None,
            )
            .await;
        assert_eq!(epoch2, 2);

        let ignored = manager
            .update_after_completion(
                "run-1",
                epoch1,
                AgentRunCompletion {
                    session: Session::new("old"),
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
                    session: Session::new("new"),
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
            .with_agent("worker", crate::AgentDefinition::new("gpt-4o-mini"))
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
            .contains("missing caller session context"));
    }

    #[tokio::test]
    async fn agent_run_tool_rejects_disallowed_target_agent() {
        let os = AgentOs::builder()
            .with_agent("worker", crate::AgentDefinition::new("gpt-4o-mini"))
            .with_agent("reviewer", crate::AgentDefinition::new("gpt-4o-mini"))
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

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::BeforeInference {
                tokio::time::sleep(Duration::from_millis(120)).await;
                step.skip_inference = true;
            }
        }
    }

    fn caller_runtime() -> carve_state::Runtime {
        let mut rt = carve_state::Runtime::new();
        rt.set(TOOL_RUNTIME_CALLER_SESSION_ID_KEY, "owner-session")
            .unwrap();
        rt.set(TOOL_RUNTIME_CALLER_AGENT_ID_KEY, "caller").unwrap();
        rt.set(TOOL_RUNTIME_CALLER_STATE_KEY, json!({"forked": true}))
            .unwrap();
        rt.set(
            TOOL_RUNTIME_CALLER_MESSAGES_KEY,
            vec![crate::Message::user("seed message")],
        )
        .unwrap();
        rt
    }

    #[tokio::test]
    async fn background_stop_then_resume_completes() {
        let os = AgentOs::builder()
            .with_agent(
                "worker",
                crate::AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SlowSkipPlugin)),
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
}
