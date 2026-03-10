# Sub-Agent Delegation

Sub-agent delegation is a built-in orchestration layer where one run can start/cancel/resume other agent runs.

## Runtime Model

Delegation is implemented through four tools:

- `agent_run`: start or resume a child run
- `task_status`: inspect child-run task state (`run_id` is also the `task_id`)
- `task_cancel`: cancel a running child run (descendants are cancelled automatically)
- `task_output`: read child run output

System behaviors (`agent_tools`, `agent_recovery`, `background_tasks`) are wired during resolve and inject usage guidance/reminders.

## Ownership and Threads

- Parent run keeps ownership in its caller thread.
- Each child run executes on its own child thread (`sub-agent-<run_id>` pattern).
- Child run records carry lineage (`parent_run_id`, `parent_thread_id`).

This keeps parent and child state/history isolated while preserving ancestry.

## State and Handle Layers

Delegation state is tracked in two layers:

1. In-memory background task manager (`BackgroundTaskManager`)
   - live status
   - cancellation token
   - owner thread checks

2. Persisted thread state (`BackgroundTaskState` at path `background_tasks`)
   - `tasks: HashMap<task_id, BackgroundTask>`
   - status (`running`, `completed`, `failed`, `cancelled`, `stopped`)
   - lightweight metadata for child thread / agent identity

The in-memory manager drives active control; persisted state supports resume/recovery semantics.

## Foreground vs Background

`agent_run(background=false)`:

- parent waits for child completion
- child progress can be forwarded to parent tool-call progress

`agent_run(background=true)`:

- child continues asynchronously
- parent gets immediate summary and may later call `agent_run` (resume/check), `task_status`, `task_cancel`, or `task_output`

## Policy and Visibility

Target-agent visibility is filtered by scope policy:

- `__agent_policy_allowed_agents`
- `__agent_policy_excluded_agents`

`AgentDefinition::allowed_agents/excluded_agents` are projected into these scope keys when absent.

## Recovery Behavior

When stale running state is detected (for example after interruption), recovery behavior can transition records and enforce explicit resume/stop decisions before replay.

## Design Tradeoff

Delegation favors explicit tool-mediated orchestration over implicit nested runtime calls, so control flow remains observable, stoppable, and policy-filterable at each boundary.
