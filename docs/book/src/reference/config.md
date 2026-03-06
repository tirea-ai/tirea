# Config

## Server Environment Variables

From `tirea-agentos-server` CLI (`crates/tirea-agentos-server/src/main.rs`):

- `AGENTOS_HTTP_ADDR` (default `127.0.0.1:8080`)
- `AGENTOS_STORAGE_DIR` (default `./threads`)
- `AGENTOS_CONFIG` (JSON config file path)
- `AGENTOS_NATS_URL` (enables NATS gateway)
- `TENSORZERO_URL` (routes model calls through TensorZero provider)

Run records are stored under `${AGENTOS_STORAGE_DIR}/runs` when using the default file run store.

## `AGENTOS_CONFIG` JSON Shape

```json
{
  "agents": [
    {
      "id": "assistant",
      "model": "gpt-4o-mini",
      "system_prompt": "You are a helpful assistant.",
      "max_rounds": 10,
      "tool_execution_mode": "parallel_streaming",
      "behavior_ids": ["tool_policy", "permission"],
      "stop_condition_specs": [
        { "type": "max_rounds", "rounds": 10 }
      ]
    }
  ]
}
```

Agent file fields:

- `id` (required)
- `model` (optional, defaults to `AgentDefinition::default().model`)
- `system_prompt` (optional, default empty string)
- `max_rounds` (optional)
- `tool_execution_mode` (optional, default `parallel_streaming`)
- `behavior_ids` (optional, default `[]`)
- `stop_condition_specs` (optional, default `[]`)

`tool_execution_mode` values:

- `sequential`
- `parallel_batch_approval`
- `parallel_streaming`

`stop_condition_specs` (`StopConditionSpec`) values:

- `max_rounds`
- `timeout`
- `token_budget`
- `consecutive_errors`
- `stop_on_tool`
- `content_match`
- `loop_detection`

## AgentDefinition Fields (Builder-Level)

`AgentDefinition` supports these orchestration fields:

- Identity/model: `id`, `model`, `system_prompt`
- Loop policy: `max_rounds`, `tool_execution_mode`
- Inference options: `chat_options`, `fallback_models`, `llm_retry_policy`
- Behavior/stop wiring: `behavior_ids`, `stop_condition_specs`, `stop_condition_ids`
- Visibility controls:
  - tools: `allowed_tools`, `excluded_tools`
  - skills: `allowed_skills`, `excluded_skills`
  - delegated agents: `allowed_agents`, `excluded_agents`

## Scope Keys (RunConfig)

Runtime policy filters are stored in `RunConfig` keys:

- `__agent_policy_allowed_tools`
- `__agent_policy_excluded_tools`
- `__agent_policy_allowed_skills`
- `__agent_policy_excluded_skills`
- `__agent_policy_allowed_agents`
- `__agent_policy_excluded_agents`
