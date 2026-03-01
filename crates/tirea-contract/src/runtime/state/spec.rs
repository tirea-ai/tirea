use serde_json::Value;
use std::any::TypeId;
use std::fmt;
use tirea_state::{
    apply_patch, get_at_path, parse_path, Op, Patch, Path, State, TireaError, TireaResult,
    TrackedPatch,
};

type ApplyFn = Box<dyn FnOnce(&Value) -> TireaResult<Patch> + Send>;

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

/// Commutative state action (data-plane) that can be merged without ordering.
///
/// These actions are path-based and intentionally avoid binding to a concrete
/// `StateSpec` type, making them suitable for parallel aggregation paths.
#[derive(Debug, Clone, PartialEq)]
pub enum CommutativeAction {
    /// Add `delta` to an integer value at `path` (default: `0` when missing).
    CounterAdd {
        path: String,
        delta: i64,
        scope: StateScope,
    },
    /// Union values into an array at `path` (set semantics, preserves order of first appearance).
    SetUnion {
        path: String,
        values: Vec<Value>,
        scope: StateScope,
    },
    /// Set integer at `path` to max(current, value) (missing path treated as unset).
    MaxI64 {
        path: String,
        value: i64,
        scope: StateScope,
    },
    /// Monotonic flag: once enabled it stays `true`.
    EnableFlag { path: String, scope: StateScope },
    /// Remove values from an array at `path` (set semantics).
    SetRemove {
        path: String,
        values: Vec<Value>,
        scope: StateScope,
    },
    /// Put entries into a map at `path` (per-key granularity).
    /// `entries` must be a JSON object.
    MapPut {
        path: String,
        entries: Value,
        scope: StateScope,
    },
    /// Remove keys from a map at `path`.
    MapRemove {
        path: String,
        keys: Vec<String>,
        scope: StateScope,
    },
    /// Set integer at `path` to min(current, value) (missing path treated as unset).
    MinI64 {
        path: String,
        value: i64,
        scope: StateScope,
    },
}

impl CommutativeAction {
    /// Logical label for diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::CounterAdd { .. } => "commutative_counter_add",
            Self::SetUnion { .. } => "commutative_set_union",
            Self::MaxI64 { .. } => "commutative_max_i64",
            Self::EnableFlag { .. } => "commutative_enable_flag",
            Self::SetRemove { .. } => "commutative_set_remove",
            Self::MapPut { .. } => "commutative_map_put",
            Self::MapRemove { .. } => "commutative_map_remove",
            Self::MinI64 { .. } => "commutative_min_i64",
        }
    }

    /// Scope for this action.
    pub fn scope(&self) -> StateScope {
        match self {
            Self::CounterAdd { scope, .. }
            | Self::SetUnion { scope, .. }
            | Self::MaxI64 { scope, .. }
            | Self::EnableFlag { scope, .. }
            | Self::SetRemove { scope, .. }
            | Self::MapPut { scope, .. }
            | Self::MapRemove { scope, .. }
            | Self::MinI64 { scope, .. } => *scope,
        }
    }

    /// Merge two commutative actions of the same kind/path/scope.
    ///
    /// Returns `None` when the two actions are not merge-compatible.
    pub fn merge(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (
                Self::CounterAdd {
                    path: l_path,
                    delta: l_delta,
                    scope: l_scope,
                },
                Self::CounterAdd {
                    path: r_path,
                    delta: r_delta,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => Some(Self::CounterAdd {
                path: l_path.clone(),
                delta: l_delta.saturating_add(*r_delta),
                scope: *l_scope,
            }),
            (
                Self::SetUnion {
                    path: l_path,
                    values: l_values,
                    scope: l_scope,
                },
                Self::SetUnion {
                    path: r_path,
                    values: r_values,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => {
                let mut merged = l_values.clone();
                for value in r_values {
                    if !merged.contains(value) {
                        merged.push(value.clone());
                    }
                }
                Some(Self::SetUnion {
                    path: l_path.clone(),
                    values: merged,
                    scope: *l_scope,
                })
            }
            (
                Self::MaxI64 {
                    path: l_path,
                    value: l_value,
                    scope: l_scope,
                },
                Self::MaxI64 {
                    path: r_path,
                    value: r_value,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => Some(Self::MaxI64 {
                path: l_path.clone(),
                value: (*l_value).max(*r_value),
                scope: *l_scope,
            }),
            (
                Self::EnableFlag {
                    path: l_path,
                    scope: l_scope,
                },
                Self::EnableFlag {
                    path: r_path,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => Some(Self::EnableFlag {
                path: l_path.clone(),
                scope: *l_scope,
            }),
            (
                Self::SetRemove {
                    path: l_path,
                    values: l_values,
                    scope: l_scope,
                },
                Self::SetRemove {
                    path: r_path,
                    values: r_values,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => {
                let mut merged = l_values.clone();
                for value in r_values {
                    if !merged.contains(value) {
                        merged.push(value.clone());
                    }
                }
                Some(Self::SetRemove {
                    path: l_path.clone(),
                    values: merged,
                    scope: *l_scope,
                })
            }
            (
                Self::MapPut {
                    path: l_path,
                    entries: l_entries,
                    scope: l_scope,
                },
                Self::MapPut {
                    path: r_path,
                    entries: r_entries,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => {
                let l_obj = l_entries.as_object()?;
                let r_obj = r_entries.as_object()?;
                let mut merged = l_obj.clone();
                for (key, r_value) in r_obj {
                    if let Some(l_value) = merged.get(key) {
                        if l_value != r_value {
                            return None; // same key, different value → conflict
                        }
                        // same key, same value → deduplicate (no-op)
                    } else {
                        merged.insert(key.clone(), r_value.clone());
                    }
                }
                Some(Self::MapPut {
                    path: l_path.clone(),
                    entries: Value::Object(merged),
                    scope: *l_scope,
                })
            }
            (
                Self::MapRemove {
                    path: l_path,
                    keys: l_keys,
                    scope: l_scope,
                },
                Self::MapRemove {
                    path: r_path,
                    keys: r_keys,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => {
                let mut merged = l_keys.clone();
                for key in r_keys {
                    if !merged.contains(key) {
                        merged.push(key.clone());
                    }
                }
                Some(Self::MapRemove {
                    path: l_path.clone(),
                    keys: merged,
                    scope: *l_scope,
                })
            }
            (
                Self::MinI64 {
                    path: l_path,
                    value: l_value,
                    scope: l_scope,
                },
                Self::MinI64 {
                    path: r_path,
                    value: r_value,
                    scope: r_scope,
                },
            ) if l_path == r_path && l_scope == r_scope => Some(Self::MinI64 {
                path: l_path.clone(),
                value: (*l_value).min(*r_value),
                scope: *l_scope,
            }),
            _ => None,
        }
    }

    /// Apply this action to a snapshot and return a patch.
    pub fn apply(&self, doc: &Value) -> TireaResult<Patch> {
        match self {
            Self::CounterAdd { path, delta, .. } => {
                let parsed = parse_path(path);
                let current = get_at_path(doc, &parsed).cloned().unwrap_or(Value::Null);
                let next = match current {
                    Value::Null => *delta,
                    Value::Number(number) => {
                        let current = number.as_i64().ok_or_else(|| {
                            TireaError::invalid_operation(format!(
                                "counter_add expects integer at path '{path}'"
                            ))
                        })?;
                        current.saturating_add(*delta)
                    }
                    _ => {
                        return Err(TireaError::invalid_operation(format!(
                            "counter_add expects integer at path '{path}'"
                        )))
                    }
                };
                Ok(Patch::with_ops(vec![Op::set(
                    path_from_str(path),
                    Value::from(next),
                )]))
            }
            Self::SetUnion { path, values, .. } => {
                let parsed = parse_path(path);
                let current = get_at_path(doc, &parsed).cloned().unwrap_or(Value::Null);
                let mut merged = match current {
                    Value::Null => Vec::new(),
                    Value::Array(items) => items,
                    _ => {
                        return Err(TireaError::invalid_operation(format!(
                            "set_union expects array at path '{path}'"
                        )))
                    }
                };
                for value in values {
                    if !merged.contains(value) {
                        merged.push(value.clone());
                    }
                }
                Ok(Patch::with_ops(vec![Op::set(
                    path_from_str(path),
                    Value::Array(merged),
                )]))
            }
            Self::MaxI64 { path, value, .. } => {
                let parsed = parse_path(path);
                let current = get_at_path(doc, &parsed).cloned().unwrap_or(Value::Null);
                let next = match current {
                    Value::Null => *value,
                    Value::Number(number) => {
                        let current = number.as_i64().ok_or_else(|| {
                            TireaError::invalid_operation(format!(
                                "max_i64 expects integer at path '{path}'"
                            ))
                        })?;
                        current.max(*value)
                    }
                    _ => {
                        return Err(TireaError::invalid_operation(format!(
                            "max_i64 expects integer at path '{path}'"
                        )))
                    }
                };
                Ok(Patch::with_ops(vec![Op::set(
                    path_from_str(path),
                    Value::from(next),
                )]))
            }
            Self::EnableFlag { path, .. } => {
                let parsed = parse_path(path);
                let current = get_at_path(doc, &parsed).cloned().unwrap_or(Value::Null);
                match current {
                    Value::Null | Value::Bool(false) => Ok(Patch::with_ops(vec![Op::set(
                        path_from_str(path),
                        Value::Bool(true),
                    )])),
                    Value::Bool(true) => Ok(Patch::new()),
                    _ => Err(TireaError::invalid_operation(format!(
                        "enable_flag expects boolean at path '{path}'"
                    ))),
                }
            }
            Self::SetRemove { path, values, .. } => {
                let parsed = parse_path(path);
                let current = get_at_path(doc, &parsed).cloned().unwrap_or(Value::Null);
                match current {
                    Value::Null => Ok(Patch::new()),
                    Value::Array(items) => {
                        let filtered: Vec<Value> =
                            items.into_iter().filter(|v| !values.contains(v)).collect();
                        Ok(Patch::with_ops(vec![Op::set(
                            path_from_str(path),
                            Value::Array(filtered),
                        )]))
                    }
                    _ => Err(TireaError::invalid_operation(format!(
                        "set_remove expects array at path '{path}'"
                    ))),
                }
            }
            Self::MapPut { path, entries, .. } => {
                let obj = entries.as_object().ok_or_else(|| {
                    TireaError::invalid_operation(format!(
                        "map_put entries must be a JSON object at path '{path}'"
                    ))
                })?;
                let base_path = path_from_str(path);
                let ops: Vec<Op> = obj
                    .iter()
                    .map(|(key, value)| Op::set(base_path.clone().key(key), value.clone()))
                    .collect();
                Ok(Patch::with_ops(ops))
            }
            Self::MapRemove { path, keys, .. } => {
                let parsed = parse_path(path);
                let current = get_at_path(doc, &parsed).cloned().unwrap_or(Value::Null);
                match current {
                    Value::Null => Ok(Patch::new()),
                    Value::Object(map) => {
                        let base_path = path_from_str(path);
                        let ops: Vec<Op> = keys
                            .iter()
                            .filter(|k| map.contains_key(k.as_str()))
                            .map(|k| Op::delete(base_path.clone().key(k)))
                            .collect();
                        Ok(Patch::with_ops(ops))
                    }
                    _ => Err(TireaError::invalid_operation(format!(
                        "map_remove expects object at path '{path}'"
                    ))),
                }
            }
            Self::MinI64 { path, value, .. } => {
                let parsed = parse_path(path);
                let current = get_at_path(doc, &parsed).cloned().unwrap_or(Value::Null);
                let next = match current {
                    Value::Null => *value,
                    Value::Number(number) => {
                        let current = number.as_i64().ok_or_else(|| {
                            TireaError::invalid_operation(format!(
                                "min_i64 expects integer at path '{path}'"
                            ))
                        })?;
                        current.min(*value)
                    }
                    _ => {
                        return Err(TireaError::invalid_operation(format!(
                            "min_i64 expects integer at path '{path}'"
                        )))
                    }
                };
                Ok(Patch::with_ops(vec![Op::set(
                    path_from_str(path),
                    Value::from(next),
                )]))
            }
        }
    }
}

/// Type-erased state action that can be applied to a JSON document.
///
/// Three variants:
/// - `Typed`: Created via [`AnyStateAction::new`] which captures a concrete
///   `StateSpec` type and action. The kernel applies these after each phase hook.
/// - `Commutative`: Path-based action following commutative merge semantics.
/// - `Patch`: A pre-built [`TrackedPatch`] that bypasses the typed reducer
///   pipeline, emitted directly as a state effect.
pub enum AnyStateAction {
    /// Type-erased action targeting a specific `StateSpec` type.
    Typed {
        state_type_id: TypeId,
        state_type_name: &'static str,
        scope: StateScope,
        apply_fn: ApplyFn,
    },
    /// Merge-friendly path-based action.
    Commutative(CommutativeAction),
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
            apply_fn: Box::new(move |doc: &Value| {
                let path = parse_path(S::PATH);
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
                    path_from_str(S::PATH),
                    new_value,
                )]))
            }),
        }
    }

    /// Wrap a commutative action.
    pub fn commutative(action: CommutativeAction) -> Self {
        Self::Commutative(action)
    }

    /// Build a run-scoped integer add action.
    pub fn counter_add(path: impl Into<String>, delta: i64) -> Self {
        Self::counter_add_scoped(path, delta, StateScope::Run)
    }

    /// Build an integer add action with explicit scope.
    pub fn counter_add_scoped(path: impl Into<String>, delta: i64, scope: StateScope) -> Self {
        Self::Commutative(CommutativeAction::CounterAdd {
            path: path.into(),
            delta,
            scope,
        })
    }

    /// Build a run-scoped array union action.
    pub fn set_union(path: impl Into<String>, values: Vec<Value>) -> Self {
        Self::set_union_scoped(path, values, StateScope::Run)
    }

    /// Build an array union action with explicit scope.
    pub fn set_union_scoped(
        path: impl Into<String>,
        values: Vec<Value>,
        scope: StateScope,
    ) -> Self {
        Self::Commutative(CommutativeAction::SetUnion {
            path: path.into(),
            values,
            scope,
        })
    }

    /// Build a run-scoped max-i64 action.
    pub fn max_i64(path: impl Into<String>, value: i64) -> Self {
        Self::max_i64_scoped(path, value, StateScope::Run)
    }

    /// Build a max-i64 action with explicit scope.
    pub fn max_i64_scoped(path: impl Into<String>, value: i64, scope: StateScope) -> Self {
        Self::Commutative(CommutativeAction::MaxI64 {
            path: path.into(),
            value,
            scope,
        })
    }

    /// Build a run-scoped monotonic enable-flag action.
    pub fn enable_flag(path: impl Into<String>) -> Self {
        Self::enable_flag_scoped(path, StateScope::Run)
    }

    /// Build a monotonic enable-flag action with explicit scope.
    pub fn enable_flag_scoped(path: impl Into<String>, scope: StateScope) -> Self {
        Self::Commutative(CommutativeAction::EnableFlag {
            path: path.into(),
            scope,
        })
    }

    /// Build a run-scoped set-remove action.
    pub fn set_remove(path: impl Into<String>, values: Vec<Value>) -> Self {
        Self::set_remove_scoped(path, values, StateScope::Run)
    }

    /// Build a set-remove action with explicit scope.
    pub fn set_remove_scoped(
        path: impl Into<String>,
        values: Vec<Value>,
        scope: StateScope,
    ) -> Self {
        Self::Commutative(CommutativeAction::SetRemove {
            path: path.into(),
            values,
            scope,
        })
    }

    /// Build a run-scoped map-put action.
    pub fn map_put(path: impl Into<String>, entries: Value) -> Self {
        Self::map_put_scoped(path, entries, StateScope::Run)
    }

    /// Build a map-put action with explicit scope.
    pub fn map_put_scoped(path: impl Into<String>, entries: Value, scope: StateScope) -> Self {
        Self::Commutative(CommutativeAction::MapPut {
            path: path.into(),
            entries,
            scope,
        })
    }

    /// Build a run-scoped map-remove action.
    pub fn map_remove(path: impl Into<String>, keys: Vec<String>) -> Self {
        Self::map_remove_scoped(path, keys, StateScope::Run)
    }

    /// Build a map-remove action with explicit scope.
    pub fn map_remove_scoped(
        path: impl Into<String>,
        keys: Vec<String>,
        scope: StateScope,
    ) -> Self {
        Self::Commutative(CommutativeAction::MapRemove {
            path: path.into(),
            keys,
            scope,
        })
    }

    /// Build a run-scoped min-i64 action.
    pub fn min_i64(path: impl Into<String>, value: i64) -> Self {
        Self::min_i64_scoped(path, value, StateScope::Run)
    }

    /// Build a min-i64 action with explicit scope.
    pub fn min_i64_scoped(path: impl Into<String>, value: i64, scope: StateScope) -> Self {
        Self::Commutative(CommutativeAction::MinI64 {
            path: path.into(),
            value,
            scope,
        })
    }

    /// The [`TypeId`] of the state type this action targets.
    ///
    /// Returns `None` for non-typed actions (`Commutative` / raw `Patch`).
    pub fn state_type_id(&self) -> Option<TypeId> {
        match self {
            Self::Typed { state_type_id, .. } => Some(*state_type_id),
            Self::Commutative(_) | Self::Patch(_) => None,
        }
    }

    /// Human-readable name of the state type (for diagnostics).
    pub fn state_type_name(&self) -> &str {
        match self {
            Self::Typed {
                state_type_name, ..
            } => state_type_name,
            Self::Commutative(action) => action.label(),
            Self::Patch(_) => "raw_patch",
        }
    }

    /// Scope of the targeted state.
    pub fn scope(&self) -> StateScope {
        match self {
            Self::Typed { scope, .. } => *scope,
            Self::Commutative(action) => action.scope(),
            Self::Patch(_) => StateScope::Run,
        }
    }

    /// Apply this action to a JSON document, producing a patch.
    ///
    /// Consumes `self` since the inner closure is `FnOnce`.
    ///
    /// For `Patch` variants, the document is ignored and the inner patch is returned.
    pub fn apply(self, doc: &Value) -> TireaResult<Patch> {
        match self {
            Self::Typed { apply_fn, .. } => apply_fn(doc),
            Self::Commutative(action) => action.apply(doc),
            Self::Patch(tracked) => Ok(tracked.patch),
        }
    }

    /// If this is a raw `Patch` action, return the tracked patch directly.
    ///
    /// Used by the effect applicator to preserve source metadata.
    pub fn into_tracked_patch(self) -> Option<TrackedPatch> {
        match self {
            Self::Patch(tracked) => Some(tracked),
            Self::Typed { .. } | Self::Commutative(_) => None,
        }
    }
}

/// Reduce a batch of state actions into tracked patches with rolling snapshot semantics.
///
/// Typed actions are reduced against a snapshot that is updated after each action,
/// so sequential actions in one batch compose deterministically.
/// Raw patch actions preserve the tracked patch metadata as-is.
pub fn reduce_state_actions(
    actions: Vec<AnyStateAction>,
    base_snapshot: &Value,
    default_source: &str,
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
            typed_action => {
                let patch = typed_action.apply(&rolling_snapshot)?;
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
            Self::Commutative(action) => f
                .debug_struct("AnyStateAction::Commutative")
                .field("action", action)
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
        const PATH: &'static str = "__tool_call_states.counter";

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
        let patch = action.apply(&doc).unwrap();

        let result = apply_patch(&doc, &patch).unwrap();
        assert_eq!(result["counters"]["main"]["value"], 8);
    }

    #[test]
    fn any_state_action_reset() {
        let doc = json!({"counters": {"main": {"value": 42}}});
        let action = AnyStateAction::new::<Counter>(CounterAction::Reset);
        let patch = action.apply(&doc).unwrap();

        let result = apply_patch(&doc, &patch).unwrap();
        assert_eq!(result["counters"]["main"]["value"], 0);
    }

    #[test]
    fn any_state_action_missing_path_defaults() {
        let doc = json!({});
        let action = AnyStateAction::new::<Counter>(CounterAction::Increment(1));
        let patch = action.apply(&doc).unwrap();

        let result = apply_patch(&doc, &patch).unwrap();
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
    fn any_state_action_scope_commutative_respects_explicit_scope() {
        let action = AnyStateAction::counter_add_scoped("metrics.count", 1, StateScope::ToolCall);
        assert_eq!(action.scope(), StateScope::ToolCall);
    }

    #[test]
    fn commutative_counter_add_reduces_without_order_dependency() {
        let base = json!({"metrics": {"count": 1}});
        let actions = vec![
            AnyStateAction::counter_add("metrics.count", 2),
            AnyStateAction::counter_add("metrics.count", 3),
        ];

        let tracked = reduce_state_actions(actions, &base, "agent").unwrap();
        assert_eq!(tracked.len(), 2);

        let mut state = base;
        for patch in tracked {
            state = apply_patch(&state, patch.patch()).unwrap();
        }
        assert_eq!(state["metrics"]["count"], 6);
    }

    #[test]
    fn commutative_set_union_appends_unique_values() {
        let base = json!({"tags": ["a"]});
        let actions = vec![AnyStateAction::set_union(
            "tags",
            vec![json!("b"), json!("a"), json!("c")],
        )];

        let tracked = reduce_state_actions(actions, &base, "agent").unwrap();
        assert_eq!(tracked.len(), 1);

        let next = apply_patch(&base, tracked[0].patch()).unwrap();
        assert_eq!(next["tags"], json!(["a", "b", "c"]));
    }

    #[test]
    fn commutative_max_i64_uses_max_value() {
        let base = json!({"score": 5});
        let actions = vec![
            AnyStateAction::max_i64("score", 3),
            AnyStateAction::max_i64("score", 11),
        ];

        let tracked = reduce_state_actions(actions, &base, "agent").unwrap();
        assert_eq!(tracked.len(), 2);

        let mut next = base;
        for patch in tracked {
            next = apply_patch(&next, patch.patch()).unwrap();
        }
        assert_eq!(next["score"], json!(11));
    }

    #[test]
    fn commutative_enable_flag_is_monotonic() {
        let base = json!({"flags": {"done": true}});
        let actions = vec![AnyStateAction::enable_flag("flags.done")];
        let tracked = reduce_state_actions(actions, &base, "agent").unwrap();
        assert!(tracked.is_empty(), "enable on true should be no-op");
    }

    #[test]
    fn commutative_merge_combines_actions() {
        let left = CommutativeAction::CounterAdd {
            path: "metrics.count".to_string(),
            delta: 2,
            scope: StateScope::Run,
        };
        let right = CommutativeAction::CounterAdd {
            path: "metrics.count".to_string(),
            delta: 3,
            scope: StateScope::Run,
        };
        let merged = left.merge(&right).expect("merge compatible");
        assert_eq!(
            merged,
            CommutativeAction::CounterAdd {
                path: "metrics.count".to_string(),
                delta: 5,
                scope: StateScope::Run,
            }
        );
    }

    #[test]
    fn reduce_state_actions_uses_rolling_snapshot() {
        let base = json!({"counters": {"main": {"value": 1}}});
        let actions = vec![
            AnyStateAction::new::<Counter>(CounterAction::Increment(1)),
            AnyStateAction::new::<Counter>(CounterAction::Increment(1)),
        ];
        let tracked = reduce_state_actions(actions, &base, "agent").unwrap();
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

        let tracked =
            reduce_state_actions(vec![AnyStateAction::Patch(raw)], &base, "agent").unwrap();
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].source.as_deref(), Some("plugin:test"));
    }

    #[test]
    #[should_panic(expected = "no bound path")]
    fn any_state_action_panics_on_empty_path() {
        let _ = AnyStateAction::new::<Unbound>(());
    }

    // -- SetRemove tests --

    #[test]
    fn commutative_set_remove_filters_matching_values() {
        let base = json!({"tags": ["a", "b", "c", "d"]});
        let action = AnyStateAction::set_remove("tags", vec![json!("b"), json!("d")]);
        let patch = action.apply(&base).unwrap();
        let next = apply_patch(&base, &patch).unwrap();
        assert_eq!(next["tags"], json!(["a", "c"]));
    }

    #[test]
    fn commutative_set_remove_null_is_noop() {
        let base = json!({});
        let action = AnyStateAction::set_remove("tags", vec![json!("x")]);
        let patch = action.apply(&base).unwrap();
        assert!(patch.is_empty());
    }

    #[test]
    fn commutative_set_remove_type_error() {
        let base = json!({"tags": "not_an_array"});
        let action = AnyStateAction::set_remove("tags", vec![json!("x")]);
        let err = action.apply(&base).unwrap_err();
        assert!(err.to_string().contains("set_remove expects array"));
    }

    #[test]
    fn commutative_set_remove_merge_unions_values() {
        let left = CommutativeAction::SetRemove {
            path: "tags".into(),
            values: vec![json!("a"), json!("b")],
            scope: StateScope::Run,
        };
        let right = CommutativeAction::SetRemove {
            path: "tags".into(),
            values: vec![json!("b"), json!("c")],
            scope: StateScope::Run,
        };
        let merged = left.merge(&right).unwrap();
        match merged {
            CommutativeAction::SetRemove { values, .. } => {
                assert_eq!(values, vec![json!("a"), json!("b"), json!("c")]);
            }
            _ => panic!("expected SetRemove"),
        }
    }

    #[test]
    fn commutative_set_remove_merge_incompatible_with_set_union() {
        let left = CommutativeAction::SetRemove {
            path: "tags".into(),
            values: vec![json!("a")],
            scope: StateScope::Run,
        };
        let right = CommutativeAction::SetUnion {
            path: "tags".into(),
            values: vec![json!("b")],
            scope: StateScope::Run,
        };
        assert!(left.merge(&right).is_none());
    }

    // -- MapPut tests --

    #[test]
    fn commutative_map_put_generates_per_key_ops() {
        let base = json!({"data": {"existing": 1}});
        let action = AnyStateAction::map_put("data", json!({"new_key": 42, "another": "hello"}));
        let patch = action.apply(&base).unwrap();
        let next = apply_patch(&base, &patch).unwrap();
        assert_eq!(next["data"]["existing"], 1);
        assert_eq!(next["data"]["new_key"], 42);
        assert_eq!(next["data"]["another"], "hello");
    }

    #[test]
    fn commutative_map_put_on_null_creates_keys() {
        let base = json!({});
        let action = AnyStateAction::map_put("data", json!({"key": "value"}));
        let patch = action.apply(&base).unwrap();
        let next = apply_patch(&base, &patch).unwrap();
        assert_eq!(next["data"]["key"], "value");
    }

    #[test]
    fn commutative_map_put_merge_disjoint_keys() {
        let left = CommutativeAction::MapPut {
            path: "data".into(),
            entries: json!({"a": 1}),
            scope: StateScope::Run,
        };
        let right = CommutativeAction::MapPut {
            path: "data".into(),
            entries: json!({"b": 2}),
            scope: StateScope::Run,
        };
        let merged = left.merge(&right).unwrap();
        match merged {
            CommutativeAction::MapPut { entries, .. } => {
                assert_eq!(entries, json!({"a": 1, "b": 2}));
            }
            _ => panic!("expected MapPut"),
        }
    }

    #[test]
    fn commutative_map_put_merge_same_key_same_value_deduplicates() {
        let left = CommutativeAction::MapPut {
            path: "data".into(),
            entries: json!({"a": 1}),
            scope: StateScope::Run,
        };
        let right = CommutativeAction::MapPut {
            path: "data".into(),
            entries: json!({"a": 1}),
            scope: StateScope::Run,
        };
        let merged = left.merge(&right).unwrap();
        match merged {
            CommutativeAction::MapPut { entries, .. } => {
                assert_eq!(entries, json!({"a": 1}));
            }
            _ => panic!("expected MapPut"),
        }
    }

    #[test]
    fn commutative_map_put_merge_same_key_different_value_conflicts() {
        let left = CommutativeAction::MapPut {
            path: "data".into(),
            entries: json!({"a": 1}),
            scope: StateScope::Run,
        };
        let right = CommutativeAction::MapPut {
            path: "data".into(),
            entries: json!({"a": 2}),
            scope: StateScope::Run,
        };
        assert!(left.merge(&right).is_none());
    }

    // -- MapRemove tests --

    #[test]
    fn commutative_map_remove_deletes_existing_keys() {
        let base = json!({"data": {"a": 1, "b": 2, "c": 3}});
        let action = AnyStateAction::map_remove("data", vec!["a".into(), "c".into()]);
        let patch = action.apply(&base).unwrap();
        let next = apply_patch(&base, &patch).unwrap();
        assert_eq!(next["data"], json!({"b": 2}));
    }

    #[test]
    fn commutative_map_remove_null_is_noop() {
        let base = json!({});
        let action = AnyStateAction::map_remove("data", vec!["x".into()]);
        let patch = action.apply(&base).unwrap();
        assert!(patch.is_empty());
    }

    #[test]
    fn commutative_map_remove_ignores_missing_keys() {
        let base = json!({"data": {"a": 1}});
        let action = AnyStateAction::map_remove("data", vec!["missing".into()]);
        let patch = action.apply(&base).unwrap();
        assert!(patch.is_empty());
    }

    #[test]
    fn commutative_map_remove_type_error() {
        let base = json!({"data": [1, 2]});
        let action = AnyStateAction::map_remove("data", vec!["x".into()]);
        let err = action.apply(&base).unwrap_err();
        assert!(err.to_string().contains("map_remove expects object"));
    }

    #[test]
    fn commutative_map_remove_merge_unions_keys() {
        let left = CommutativeAction::MapRemove {
            path: "data".into(),
            keys: vec!["a".into(), "b".into()],
            scope: StateScope::Run,
        };
        let right = CommutativeAction::MapRemove {
            path: "data".into(),
            keys: vec!["b".into(), "c".into()],
            scope: StateScope::Run,
        };
        let merged = left.merge(&right).unwrap();
        match merged {
            CommutativeAction::MapRemove { keys, .. } => {
                assert_eq!(keys, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
            }
            _ => panic!("expected MapRemove"),
        }
    }

    // -- MinI64 tests --

    #[test]
    fn commutative_min_i64_uses_min_value() {
        let base = json!({"score": 10});
        let actions = vec![
            AnyStateAction::min_i64("score", 15),
            AnyStateAction::min_i64("score", 3),
        ];
        let tracked = reduce_state_actions(actions, &base, "agent").unwrap();
        let mut next = base;
        for patch in tracked {
            next = apply_patch(&next, patch.patch()).unwrap();
        }
        assert_eq!(next["score"], json!(3));
    }

    #[test]
    fn commutative_min_i64_null_uses_value() {
        let base = json!({});
        let action = AnyStateAction::min_i64("score", 42);
        let patch = action.apply(&base).unwrap();
        let next = apply_patch(&base, &patch).unwrap();
        assert_eq!(next["score"], 42);
    }

    #[test]
    fn commutative_min_i64_type_error() {
        let base = json!({"score": "not_a_number"});
        let action = AnyStateAction::min_i64("score", 5);
        let err = action.apply(&base).unwrap_err();
        assert!(err.to_string().contains("min_i64 expects integer"));
    }

    #[test]
    fn commutative_min_i64_merge_takes_min() {
        let left = CommutativeAction::MinI64 {
            path: "score".into(),
            value: 10,
            scope: StateScope::Run,
        };
        let right = CommutativeAction::MinI64 {
            path: "score".into(),
            value: 3,
            scope: StateScope::Run,
        };
        let merged = left.merge(&right).unwrap();
        assert_eq!(
            merged,
            CommutativeAction::MinI64 {
                path: "score".into(),
                value: 3,
                scope: StateScope::Run,
            }
        );
    }

    #[test]
    fn commutative_min_i64_merge_incompatible_with_max_i64() {
        let left = CommutativeAction::MinI64 {
            path: "score".into(),
            value: 10,
            scope: StateScope::Run,
        };
        let right = CommutativeAction::MaxI64 {
            path: "score".into(),
            value: 5,
            scope: StateScope::Run,
        };
        assert!(left.merge(&right).is_none());
    }
}
