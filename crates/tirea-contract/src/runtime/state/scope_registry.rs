use super::spec::{AnyStateAction, StateScope};
use tirea_state::StateSpec;
use std::any::TypeId;
use std::collections::HashMap;

/// Registry mapping `StateSpec` types to their declared [`StateScope`].
///
/// Built once at agent construction by calling
/// [`AgentBehavior::register_state_scopes`] on each behavior. The loop then
/// uses [`resolve`] to determine the scope of any [`AnyStateAction`] without
/// relying on the action carrying the scope internally.
#[derive(Debug, Clone, Default)]
pub struct StateScopeRegistry {
    typed: HashMap<TypeId, (&'static str, StateScope)>,
}

impl StateScopeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a [`StateSpec`] type with an explicit [`StateScope`].
    pub fn register<S: StateSpec>(&mut self, scope: StateScope) {
        self.typed
            .insert(TypeId::of::<S>(), (std::any::type_name::<S>(), scope));
    }

    /// Look up the scope of a registered type.
    pub fn typed_scope(&self, type_id: TypeId) -> Option<StateScope> {
        self.typed.get(&type_id).map(|(_, scope)| *scope)
    }

    /// Resolve the scope of an [`AnyStateAction`].
    ///
    /// If the action targets a registered type, returns the registered scope.
    /// Otherwise falls back to [`AnyStateAction::scope`].
    pub fn resolve(&self, action: &AnyStateAction) -> StateScope {
        if let Some(type_id) = action.state_type_id() {
            if let Some(scope) = self.typed_scope(type_id) {
                return scope;
            }
        }
        action.scope()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use tirea_state::{DocCell, PatchSink, Path, State, TireaResult};

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct RunScoped {
        value: i64,
    }

    struct RunScopedRef;

    impl State for RunScoped {
        type Ref<'a> = RunScopedRef;
        const PATH: &'static str = "run_scoped";

        fn state_ref<'a>(_: &'a DocCell, _: Path, _: PatchSink<'a>) -> Self::Ref<'a> {
            RunScopedRef
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

    impl StateSpec for RunScoped {
        type Action = ();
        fn reduce(&mut self, _: ()) {}
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct ToolScoped {
        value: i64,
    }

    struct ToolScopedRef;

    impl State for ToolScoped {
        type Ref<'a> = ToolScopedRef;
        const PATH: &'static str = "tool_scoped";

        fn state_ref<'a>(_: &'a DocCell, _: Path, _: PatchSink<'a>) -> Self::Ref<'a> {
            ToolScopedRef
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

    impl StateSpec for ToolScoped {
        type Action = ();
        fn reduce(&mut self, _: ()) {}
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = StateScopeRegistry::new();
        reg.register::<RunScoped>(StateScope::Run);
        reg.register::<ToolScoped>(StateScope::ToolCall);

        assert_eq!(
            reg.typed_scope(TypeId::of::<RunScoped>()),
            Some(StateScope::Run)
        );
        assert_eq!(
            reg.typed_scope(TypeId::of::<ToolScoped>()),
            Some(StateScope::ToolCall)
        );
    }

    #[test]
    fn unregistered_type_returns_none() {
        let reg = StateScopeRegistry::new();
        assert_eq!(reg.typed_scope(TypeId::of::<RunScoped>()), None);
    }

    #[test]
    fn resolve_falls_back_to_action_scope() {
        let reg = StateScopeRegistry::new();
        let action = AnyStateAction::new::<RunScoped>(());
        assert_eq!(reg.resolve(&action), StateScope::Run);
    }

    #[test]
    fn resolve_uses_registered_scope() {
        let mut reg = StateScopeRegistry::new();
        reg.register::<ToolScoped>(StateScope::ToolCall);
        let action = AnyStateAction::new::<ToolScoped>(());
        assert_eq!(reg.resolve(&action), StateScope::ToolCall);
    }
}
