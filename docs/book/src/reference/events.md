# Events

The agent loop emits `AgentEvent` values as it executes. Events are streamed to
clients via SSE and consumed by protocol encoders.

## AgentEvent

All variants are tagged with `event_type` in their JSON serialization
(`#[serde(tag = "event_type", rename_all = "snake_case")]`).

```rust,ignore
pub enum AgentEvent {
    RunStart {
        thread_id: String,
        run_id: String,
        parent_run_id: Option<String>,    // omitted when None
    },

    RunFinish {
        thread_id: String,
        run_id: String,
        result: Option<Value>,            // omitted when None
        termination: TerminationReason,
    },

    TextDelta { delta: String },

    ReasoningDelta { delta: String },

    ToolCallStart { id: String, name: String },

    ToolCallDelta { id: String, args_delta: String },

    ToolCallReady {
        id: String,
        name: String,
        arguments: Value,
    },

    ToolCallDone {
        id: String,
        message_id: String,
        result: ToolResult,
        outcome: ToolCallOutcome,
    },

    ToolCallStreamDelta {
        id: String,
        name: String,
        delta: String,
    },

    ReasoningEncryptedValue { encrypted_value: String },

    MessagesSnapshot { messages: Vec<Value> },

    ActivitySnapshot {
        message_id: String,
        activity_type: String,
        content: Value,
        replace: Option<bool>,            // omitted when None
    },

    ActivityDelta {
        message_id: String,
        activity_type: String,
        patch: Vec<Value>,
    },

    ToolCallResumed { target_id: String, result: Value },

    StepStart { message_id: String },

    StepEnd,

    InferenceComplete {
        model: String,
        usage: Option<TokenUsage>,        // omitted when None
        duration_ms: u64,
    },

    StateSnapshot { snapshot: Value },

    StateDelta { delta: Vec<Value> },

    Error {
        message: String,
        code: Option<String>,             // omitted when None
    },
}
```

**Crate path:** `awaken::contract::event::AgentEvent`

### Helper

```rust,ignore
impl AgentEvent {
    /// Extract the response text from a RunFinish result value.
    pub fn extract_response(result: &Option<Value>) -> String
}
```

## SSE Wire Format

Events are sent over HTTP as Server-Sent Events. Each frame uses the SSE `id`
field as a monotonically increasing sequence number for resumability:

```
id: {seq}
data: {json}

```

Where `{json}` is the JSON-serialized `AgentEvent`. The sequence counter starts
at 0 and increments by 1 for each event emitted within a run.

The server also sends periodic keep-alive comment lines (`: heartbeat`) when the
stream is idle to prevent client/proxy timeouts during long inference pauses.
These comment lines carry no event data and must be ignored by clients.

Implemented via `format_sse_data_with_id` in `awaken-server::http_sse`.

## Run Inputs

To push messages into a run, POST to `/v1/runs/{run_id}/inputs`. The request
body is deserialized as `PushRunInputsPayload`:

```rust,ignore
struct PushRunInputsPayload {
    messages: Vec<RunMessage>,
}

struct RunMessage {
    role: String,
    content: String,
}
```

At least one message is required. The server returns `202 Accepted` on success.

## RunOutput

Type alias for the event stream returned by a run:

```rust,ignore
pub type RunOutput = futures::stream::BoxStream<'static, AgentEvent>;
```

## TerminationReason

Why a run terminated. Serialized with `#[serde(tag = "type", content = "value",
rename_all = "snake_case")]`.

- Unit variants (no payload) serialize as `{ "type": "natural_end" }` — no
  `"value"` key is emitted.
- Tuple variants serialize as `{ "type": "stopped", "value": { ... } }`.

```rust,ignore
pub enum TerminationReason {
    NaturalEnd,
    BehaviorRequested,
    Stopped(StoppedReason),
    Cancelled,
    Blocked(String),
    Suspended,
    Error(String),
}
```

## StoppedReason

Payload carried by `TerminationReason::Stopped`.

```rust,ignore
pub struct StoppedReason {
    pub code: String,
    pub detail: Option<String>,    // omitted when None
}
```

## ToolCallOutcome

```rust,ignore
pub enum ToolCallOutcome {
    Succeeded,
    Failed,
    Suspended,
}
```

## ToolResult

Result of a tool execution, carried in `ToolCallDone`. Defined in
`awaken::contract::tool::ToolResult`.

```rust,ignore
pub struct ToolResult {
    pub tool_name: String,
    pub status: ToolStatus,        // "success" | "pending" | "error"
    pub data: Value,
    pub message: Option<String>,
    pub suspension: Option<Box<SuspendTicket>>,   // omitted when None
    pub metadata: HashMap<String, Value>,          // omitted when empty
}
```

## TokenUsage

```rust,ignore
pub struct TokenUsage {
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub cache_read_tokens: Option<i32>,
    pub cache_creation_tokens: Option<i32>,
    pub thinking_tokens: Option<i32>,
}
```

All fields are omitted from JSON when `None`. `TokenUsage::default()` produces
all `None` values.

## Related

- [Run Lifecycle and Phases](../explanation/run-lifecycle-and-phases.md)
