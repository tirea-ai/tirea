# Config

## Server Environment Variables

From `tirea-agentos-server` CLI:

- `AGENTOS_HTTP_ADDR` (default `127.0.0.1:8080`)
- `AGENTOS_STORAGE_DIR` (default `./threads`)
- `AGENTOS_CONFIG` (path to JSON config)
- `AGENTOS_NATS_URL` (enable NATS gateway)
- `TENSORZERO_URL` (route model calls via TensorZero)

## Agent Definition Fields

Common `AgentDefinition` controls:

- `id`, `model`, `system_prompt`
- `max_rounds`, `parallel_tools`
- `plugin_ids`
- `allowed_tools` / `excluded_tools`
- `stop_condition_specs` / `stop_condition_ids`
