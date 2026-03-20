# ADR-0003: Configuration and Plugin Activation

- **Status**: Accepted
- **Date**: 2026-03-21

## Context

awaken's `Plugin` trait unifies state and behavior in a single `register()` call — simpler than tirea's separated model (AgentBehavior + `declare_plugin_states!` + multiple registration methods). Two capabilities are missing:

1. **Per-agent plugin activation** — all installed plugins' hooks run for every `run_phase`.
2. **Typed plugin configuration** — no way to set config externally without `StateSlot` mutation.

Configuration differs from state: it is whole-replacement (not reduced), comes from outside the runtime, and does not participate in revision tracking. However, some configuration decisions (handoff, dynamic skill discovery) happen inside hooks that can only write state. This means configuration must be resolvable from both external APIs and state.

## Decisions

### D1: `ConfigSlot` trait — typed plugin configuration

Parallel to `StateSlot` but without `Update` / `apply`. Values are whole-replacement, stored in a `ConfigMap` (separate `TypedMap` with distinct marker). Plugins register config slots via `PluginRegistrar::register_config::<C>()`.

### D2: Multi-source configuration resolution

Configuration is resolved from multiple sources at each execution boundary, in precedence order:

1. `RunOverrides` — per-call, not persisted
2. `ActiveConfig` — runtime baseline, set via `configure()` closure
3. Active profile config — from profile referenced by state or ActiveConfig
4. `OsConfig.defaults` — global defaults
5. `C::Value::default()` — type default

`active_plugins` follows the same multi-source merge: state-derived profile override + runtime baseline + per-call deltas.

**Why**: Plugins inside hooks can only write state, not call `configure()`. Handoff writes `active_profile` to state; the next boundary resolves it.

### D3: `ActiveConfig` as runtime baseline, not sole truth

`ActiveConfig` holds `active_plugins` + `config: ConfigMap`. Modified via `runtime.configure()` closure. Named profiles (`AgentProfile`) are convenience presets loaded into `ActiveConfig`. It is one input to resolution (D2), not the sole truth source.

### D4: Resolve at boundary

Resolution produces a short-lived `ResolvedPhaseConfig` (active plugins + `Arc<ConfigMap>`) at the start of each `run_phase`. All hooks within the same phase see the same resolved config. Changes to `ActiveConfig` or state mid-phase take effect at the next boundary.

`resolve_config()` is exposed as a public method so the upper-layer agent loop can reuse the same resolution logic at other boundaries (before inference, before tool execution).

### D5: State-derived configuration via `ActiveProfileOverride`

A built-in `StateSlot` that plugins write via `StateCommand` to override the active profile. When set, `resolve_config` looks up the referenced profile in `OsConfig.profiles` and merges it into the resolution chain. Enables handoff without calling `configure()`.

### D6: Per-agent hook filtering

GATHER filters hooks by resolved `active_plugins`. EXECUTE does not filter action handlers — they are global capabilities. State slots are global and readable by any hook.

### D7: Hook ordering not required

Registration-order execution is sufficient. Cross-plugin coordination flows through the action queue (ADR-0001 D1). Topological sorting can be added later via `after`/`before` constraints on `register_phase_hook()` without changing the `Plugin` trait.

### D8: `ConfigView` deferred

Cross-source merge projections (config + state + memory -> effective value) are plugin-defined functions for now. A framework-level `ConfigView` trait will be introduced when 3-5 plugins show repeated patterns.

## Consequences

### To be implemented

- `ConfigSlot` trait + `ConfigMap` + `PluginRegistrar::register_config::<C>()`
- `ActiveConfig`, `OsConfig`, `AgentProfile`, `RunOverrides`, `ResolvedPhaseConfig`
- `ActiveProfileOverride` built-in `StateSlot`
- `PhaseRuntime::configure()`, `resolve_config()`, `run_phase_with()`
- `PhaseContext` gains `config: Arc<ConfigMap>` + `config::<C>()` accessor
- `PhaseHookRegistration` gains `plugin_id: String`; hook filtering in `run_phase`

### Deferred

- `ConfigView` trait (D8)
- Dynamic capability injection (MCP/Skills)
- State scope (ADR-0001)
