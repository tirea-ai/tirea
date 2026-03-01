use super::scope_context::ScopeContext;
use serde_json::Value;
use std::any::TypeId;
use std::fmt;
use tirea_state::{
    apply_patch, get_at_path, parse_path, Op, Patch, Path, State, TireaResult, TrackedPatch,
};

type ReduceFn = Box<dyn FnOnce(&Value, &str) -> TireaResult<Patch> + Send>;

/// Runtime scope where a state is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateScope {
    /// State that lives for the entire run.
    Run,
    /// State that is scoped to a single tool call.
    ToolCall,
}

/// Extends [`State`] with a typed action and a pure reducer.
///
/// Implementors define what actions their state accepts and how the state
/// transitions in response. The kernel applies actions via [`AnyStateAction`]
/// without knowing the concrete types.
///
/// # Example
///
/// ```ignore
/// impl StateSpec for Counter {
///     type Action = CounterAction;
///     fn reduce(&mut self, action: CounterAction) {
///         match action {
///             CounterAction::Increment(n) => self.value += n,
///             CounterAction::Reset => self.value = 0,
///         }
///     }
/// }
/// ```
pub trait StateSpec: State + Sized + Send + 'static {
    /// The action type accepted by this state.
    type Action: Send + 'static;

    /// Runtime scope for this state.
    const SCOPE: StateScope = StateScope::Run;

    /// Pure reducer: apply an action to produce the next state.
    fn reduce(&mut self, action: Self::Action);
}


/// Type-erased state action that can be applied to a JSON document.
///
/// Two variants:
/// - `Typed`: Created via [`AnyStateAction::new`] which captures a concrete
///   `StateSpec` type and action. The kernel applies these after each phase hook.
/// - `Patch`: A pre-built [`TrackedPatch`] that bypasses the typed reducer
///   pipeline, emitted directly as a state effect.
pub enum AnyStateAction {
    /// Type-erased action targeting a specific `StateSpec` type.
    Typed {
        state_type_id: TypeId,
        state_type_name: &'static str,
        scope: StateScope,
        base_path: &'static str,
        /// When set, overrides the `ScopeContext` call_id for path resolution.
        /// Used by recovery/framework-internal scenarios that must target a
        /// specific call_id without a live `ScopeContext`.
        call_id_override: Option<String>,
        reduce_fn: ReduceFn,
    },
    /// Pre-built tracked patch emitted directly as a state effect.
    Patch(TrackedPatch),
}

impl AnyStateAction {
    /// Create a type-erased action targeting state `S`.
    ///
    /// # Panics
    ///
    /// Panics if `S::PATH` is empty (state must have a bound path).
    pub fn new<S: StateSpec>(action: S::Action) -> Self {
        assert!(
            !S::PATH.is_empty(),
            "StateSpec type has no bound path; cannot create AnyStateAction"
        );

        Self::Typed {
            state_type_id: TypeId::of::<S>(),
            state_type_name: std::any::type_name::<S>(),
            scope: S::SCOPE,
            base_path: S::PATH,
            call_id_override: None,
            reduce_fn: Self::make_reduce_fn::<S>(action),
        }
    }

    /// Create a type-erased action with an explicit call_id override.
    ///
    /// Used for recovery/framework-internal scenarios where the action must
    /// target a specific tool call id without relying on the ambient `ScopeContext`.
    ///
    /// # Panics
    ///
    /// Panics if `S::PATH` is empty.
    pub fn new_for_call<S: StateSpec>(action: S::Action, call_id: impl Into<String>) -> Self {
        assert!(
            !S::PATH.is_empty(),
            "StateSpec type has no bound path; cannot create AnyStateAction"
        );

        Self::Typed {
            state_type_id: TypeId::of::<S>(),
            state_type_name: std::any::type_name::<S>(),
            scope: S::SCOPE,
            base_path: S::PATH,
            call_id_override: Some(call_id.into()),
            reduce_fn: Self::make_reduce_fn::<S>(action),
        }
    }

    fn make_reduce_fn<S: StateSpec>(action: S::Action) -> ReduceFn {
        Box::new(move |doc: &Value, actual_path: &str| {
            let path = parse_path(actual_path);
            let sub_doc = get_at_path(doc, &path).cloned().unwrap_or(Value::Null);
            // When the path doesn't exist (Null) and from_value fails,
            // fall back to an empty object. This handles derive(State) structs
            // whose #[serde(default)] fields can deserialize from `{}` but not
            // from `null` (serde_json rejects null for struct types).
            let mut state = S::from_value(&sub_doc).or_else(|first_err| {
                if sub_doc.is_null() {
                    S::from_value(&Value::Object(Default::default())).map_err(|_| first_err)
                } else {
                    Err(first_err)
                }
            })?;
            state.reduce(action);
            let new_value = state.to_value()?;
            Ok(Patch::with_ops(vec![Op::set(
                path_from_str(actual_path),
                new_value,
            )]))
        })
    }

    /// The [`TypeId`] of the state type this action targets.
    ///
    /// Returns `None` for raw `Patch` actions.
    pub fn state_type_id(&self) -> Option<TypeId> {
        match self {
            Self::Typed { state_type_id, .. } => Some(*state_type_id),
            Self::Patch(_) => None,
        }
    }

    /// Human-readable name of the state type (for diagnostics).
    pub fn state_type_name(&self) -> &str {
        match self {
            Self::Typed {
                state_type_name, ..
            } => state_type_name,
            Self::Patch(_) => "raw_patch",
        }
    }

    /// Scope of the targeted state.
    pub fn scope(&self) -> StateScope {
        match self {
            Self::Typed { scope, .. } => *scope,
            Self::Patch(_) => StateScope::Run,
        }
    }

    /// If this is a raw `Patch` action, return the tracked patch directly.
    ///
    /// Used by the effect applicator to preserve source metadata.
    pub fn into_tracked_patch(self) -> Option<TrackedPatch> {
        match self {
            Self::Patch(tracked) => Some(tracked),
            Self::Typed { .. } => None,
        }
    }
}

/// Reduce a batch of state actions into tracked patches with rolling snapshot semantics.
///
/// Typed actions are reduced against a snapshot that is updated after each action,
/// so sequential actions in one batch compose deterministically.
/// Raw patch actions preserve the tracked patch metadata as-is.
///
/// `scope_ctx` controls how `ToolCall`-scoped actions are routed to per-call
/// namespaces. For non-tool phases, pass `ScopeContext::run()`.
pub fn reduce_state_actions(
    actions: Vec<AnyStateAction>,
    base_snapshot: &Value,
    default_source: &str,
    scope_ctx: &ScopeContext,
) -> TireaResult<Vec<TrackedPatch>> {
    let mut rolling_snapshot = base_snapshot.clone();
    let mut tracked_patches = Vec::new();

    for action in actions {
        match action {
            AnyStateAction::Patch(tracked) => {
                if tracked.patch().is_empty() {
                    continue;
                }
                rolling_snapshot = apply_patch(&rolling_snapshot, tracked.patch())?;
                tracked_patches.push(tracked);
            }
            AnyStateAction::Typed {
                scope,
                base_path,
                call_id_override,
                reduce_fn,
                ..
            } => {
                // Resolve actual storage path: call_id_override takes priority,
                // then fall back to the ambient scope_ctx.
                let actual_path = if let Some(ref cid) = call_id_override {
                    let override_ctx = ScopeContext::for_call(cid.as_str());
                    override_ctx.resolve_path(scope, base_path)
                } else {
                    scope_ctx.resolve_path(scope, base_path)
                };
                let patch = reduce_fn(&rolling_snapshot, &actual_path)?;
                if patch.is_empty() {
                    continue;
                }
                rolling_snapshot = apply_patch(&rolling_snapshot, &patch)?;
                tracked_patches.push(TrackedPatch::new(patch).with_source(default_source));
            }
        }
    }

    Ok(tracked_patches)
}

impl fmt::Debug for AnyStateAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Typed {
                state_type_name,
                state_type_id,
                scope,
                ..
            } => f
                .debug_struct("AnyStateAction::Typed")
                .field("state", state_type_name)
                .field("type_id", state_type_id)
                .field("scope", scope)
                .finish(),
            Self::Patch(tracked) => f
                .debug_struct("AnyStateAction::Patch")
                .field("source", &tracked.source)
                .finish(),
        }
    }
}

/// Convert a dot-separated path string to a `Path` for use in `Op::set`.
fn path_from_str(s: &str) -> Path {
    let mut path = Path::root();
    for seg in s.split('.') {
        if !seg.is_empty() {
            path = path.key(seg);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use tirea_state::{apply_patch, DocCell, PatchSink, Path as TPath};

    // -- Manual State + StateSpec impl for testing --

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Counter {
        value: i64,
    }

    impl Default for Counter {
        fn default() -> Self {
            Self { value: 0 }
        }
    }

    struct CounterRef;

    impl State for Counter {
        type Ref<'a> = CounterRef;
        const PATH: &'static str = "counters.main";

        fn state_ref<'a>(_: &'a DocCell, _: TPath, _: PatchSink<'a>) -> Self::Ref<'a> {
            CounterRef
        }

        fn from_value(value: &Value) -> TireaResult<Self> {
            if value.is_null() {
                return Ok(Self::default());
            }
            serde_json::from_value(value.clone()).map_err(tirea_state::TireaError::Serialization)
        }

        fn to_value(&self) -> TireaResult<Value> {
            serde_json::to_value(self).map_err(tirea_state::TireaError::Serialization)
        }
    }

    #[derive(Debug)]
    enum CounterAction {
        Increment(i64),
        Reset,
    }

    impl StateSpec for Counter {
        type Action = CounterAction;

        fn reduce(&mut self, action: CounterAction) {
            match action {
                CounterAction::Increment(n) => self.value += n,
                CounterAction::Reset => self.value = 0,
            }
        }
    }

    // -- No-path state for panic test --

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Unbound {
        x: i64,
    }

    struct UnboundRef;

    impl State for Unbound {
        type Ref<'a> = UnboundRef;
        // PATH defaults to "" (no bound path)

        fn state_ref<'a>(_: &'a DocCell, _: TPath, _: PatchSink<'a>) -> Self::Ref<'a> {
            UnboundRef
        }

        fn from_value(value: &Value) -> TireaResult<Self> {
            serde_json::from_value(value.clone()).map_err(tirea_state::TireaError::Serialization)
        }

        fn to_value(&self) -> TireaResult<Value> {
            serde_json::to_value(self).map_err(tirea_state::TireaError::Serialization)
        }
    }

    impl StateSpec for Unbound {
        type Action = ();
        fn reduce(&mut self, _: ()) {}
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct ToolScopedCounter {
        value: i64,
    }

    struct ToolScopedCounterRef;

    impl State for ToolScopedCounter {
        type Ref<'a> = ToolScopedCounterRef;
        const PATH: &'static str = "tool_counter";

        fn state_ref<'a>(_: &'a DocCell, _: TPath, _: PatchSink<'a>) -> Self::Ref<'a> {
            ToolScopedCounterRef
        }

        fn from_value(value: &Value) -> TireaResult<Self> {
            if value.is_null() {
                return Ok(Self::default());
            }
            serde_json::from_value(value.clone()).map_err(tirea_state::TireaError::Serialization)
        }

        fn to_value(&self) -> TireaResult<Value> {
            serde_json::to_value(self).map_err(tirea_state::TireaError::Serialization)
        }
    }

    impl StateSpec for ToolScopedCounter {
        type Action = CounterAction;
        const SCOPE: StateScope = StateScope::ToolCall;

        fn reduce(&mut self, action: Self::Action) {
            match action {
                CounterAction::Increment(n) => self.value += n,
                CounterAction::Reset => self.value = 0,
            }
        }
    }

    // -- Tests --

    #[test]
    fn any_state_action_increment() {
        let doc = json!({"counters": {"main": {"value": 5}}});
        let action = AnyStateAction::new::<Counter>(CounterAction::Increment(3));
        let patch = reduce_state_actions(
            vec![action],
            &doc,
            "test",
            &ScopeContext::run(),
        )
        .unwrap();
        let result = apply_patch(&doc, patch[0].patch()).unwrap();
        assert_eq!(result["counters"]["main"]["value"], 8);
    }

    #[test]
    fn any_state_action_reset() {
        let doc = json!({"counters": {"main": {"value": 42}}});
        let action = AnyStateAction::new::<Counter>(CounterAction::Reset);
        let patch = reduce_state_actions(
            vec![action],
            &doc,
            "test",
            &ScopeContext::run(),
        )
        .unwrap();
        let result = apply_patch(&doc, patch[0].patch()).unwrap();
        assert_eq!(result["counters"]["main"]["value"], 0);
    }

    #[test]
    fn any_state_action_missing_path_defaults() {
        let doc = json!({});
        let action = AnyStateAction::new::<Counter>(CounterAction::Increment(1));
        let patch = reduce_state_actions(
            vec![action],
            &doc,
            "test",
            &ScopeContext::run(),
        )
        .unwrap();
        let result = apply_patch(&doc, patch[0].patch()).unwrap();
        assert_eq!(result["counters"]["main"]["value"], 1);
    }

    #[test]
    fn any_state_action_label() {
        let action = AnyStateAction::new::<Counter>(CounterAction::Increment(1));
        assert!(action.state_type_name().contains("Counter"));
    }

    #[test]
    fn any_state_action_debug() {
        let action = AnyStateAction::new::<Counter>(CounterAction::Increment(1));
        let debug = format!("{action:?}");
        assert!(debug.contains("AnyStateAction"));
        assert!(debug.contains("Counter"));
    }

    #[test]
    fn any_state_action_state_type_id() {
        let action = AnyStateAction::new::<Counter>(CounterAction::Increment(1));
        assert_eq!(action.state_type_id(), Some(TypeId::of::<Counter>()));
    }

    #[test]
    fn any_state_action_scope_defaults_to_run() {
        let action = AnyStateAction::new::<Counter>(CounterAction::Increment(1));
        assert_eq!(action.scope(), StateScope::Run);
    }

    #[test]
    fn any_state_action_scope_tool_call() {
        let action = AnyStateAction::new::<ToolScopedCounter>(CounterAction::Increment(1));
        assert_eq!(action.scope(), StateScope::ToolCall);
    }

    #[test]
    fn reduce_state_actions_uses_rolling_snapshot() {
        let base = json!({"counters": {"main": {"value": 1}}});
        let actions = vec![
            AnyStateAction::new::<Counter>(CounterAction::Increment(1)),
            AnyStateAction::new::<Counter>(CounterAction::Increment(1)),
        ];
        let tracked =
            reduce_state_actions(actions, &base, "agent", &ScopeContext::run()).unwrap();
        assert_eq!(tracked.len(), 2);

        let mut state = base.clone();
        for patch in tracked {
            state = apply_patch(&state, patch.patch()).unwrap();
        }
        assert_eq!(state["counters"]["main"]["value"], 3);
    }

    #[test]
    fn reduce_state_actions_preserves_raw_patch_source() {
        let base = json!({});
        let raw = TrackedPatch::new(Patch::with_ops(vec![Op::set(
            path_from_str("debug.raw"),
            json!(true),
        )]))
        .with_source("plugin:test");

        let tracked = reduce_state_actions(
            vec![AnyStateAction::Patch(raw)],
            &base,
            "agent",
            &ScopeContext::run(),
        )
        .unwrap();
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].source.as_deref(), Some("plugin:test"));
    }

    #[test]
    #[should_panic(expected = "no bound path")]
    fn any_state_action_panics_on_empty_path() {
        let _ = AnyStateAction::new::<Unbound>(());
    }

    #[test]
    fn reduce_tool_call_scoped_action_routes_to_call_namespace() {
        let base = json!({});
        let scope_ctx = ScopeContext::for_call("call_42");
        let actions = vec![AnyStateAction::new::<ToolScopedCounter>(
            CounterAction::Increment(5),
        )];
        let tracked =
            reduce_state_actions(actions, &base, "test", &scope_ctx).unwrap();
        assert_eq!(tracked.len(), 1);

        let result = apply_patch(&base, tracked[0].patch()).unwrap();
        assert_eq!(
            result["__tool_call_scope"]["call_42"]["tool_counter"]["value"],
            5
        );
    }

    #[test]
    fn reduce_run_scoped_action_ignores_call_context() {
        let base = json!({});
        let scope_ctx = ScopeContext::for_call("call_42");
        let actions = vec![AnyStateAction::new::<Counter>(CounterAction::Increment(7))];
        let tracked =
            reduce_state_actions(actions, &base, "test", &scope_ctx).unwrap();

        let result = apply_patch(&base, tracked[0].patch()).unwrap();
        assert_eq!(result["counters"]["main"]["value"], 7);
        assert!(result.get("__tool_call_scope").is_none());
    }

    #[test]
    fn new_for_call_overrides_scope_ctx() {
        let base = json!({});
        let scope_ctx = ScopeContext::for_call("ambient_call");
        let actions = vec![AnyStateAction::new_for_call::<ToolScopedCounter>(
            CounterAction::Increment(3),
            "override_call",
        )];
        let tracked =
            reduce_state_actions(actions, &base, "test", &scope_ctx).unwrap();

        let result = apply_patch(&base, tracked[0].patch()).unwrap();
        // Should use the override call_id, not the ambient one
        assert_eq!(
            result["__tool_call_scope"]["override_call"]["tool_counter"]["value"],
            3
        );
        assert!(result["__tool_call_scope"].get("ambient_call").is_none());
    }

    #[test]
    #[should_panic(expected = "no bound path")]
    fn new_for_call_panics_on_empty_path() {
        let _ = AnyStateAction::new_for_call::<Unbound>((), "call_1");
    }
}
