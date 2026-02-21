//! State trait for typed state access.
//!
//! The `State` trait provides a unified interface for typed access to JSON documents.
//! It is typically implemented via the derive macro `#[derive(State)]`.

use crate::{DocCell, Op, Patch, Path, TireaResult, TrackedPatch};
use serde_json::Value;
use std::sync::{Arc, Mutex};

type CollectHook<'a> = Arc<dyn Fn(&Op) -> TireaResult<()> + Send + Sync + 'a>;

/// Collector for patch operations.
///
/// `PatchSink` collects operations that will be combined into a `Patch`.
/// It is used internally by `StateRef` types to automatically collect
/// all state modifications.
///
/// # Thread Safety
///
/// `PatchSink` uses a `Mutex` internally to support async contexts.
/// In single-threaded usage, the lock overhead is minimal.
pub struct PatchSink<'a> {
    ops: Option<&'a Mutex<Vec<Op>>>,
    on_collect: Option<CollectHook<'a>>,
}

impl<'a> PatchSink<'a> {
    /// Create a new PatchSink wrapping a Mutex.
    #[doc(hidden)]
    pub fn new(ops: &'a Mutex<Vec<Op>>) -> Self {
        Self {
            ops: Some(ops),
            on_collect: None,
        }
    }

    /// Create a new PatchSink with a collect hook.
    ///
    /// The hook is invoked after each operation is collected.
    #[doc(hidden)]
    pub fn new_with_hook(ops: &'a Mutex<Vec<Op>>, hook: CollectHook<'a>) -> Self {
        Self {
            ops: Some(ops),
            on_collect: Some(hook),
        }
    }

    /// Create a child sink that shares the same collector and hook.
    ///
    /// Nested state refs use this so write-through behavior is preserved.
    #[doc(hidden)]
    pub fn child(&self) -> Self {
        Self {
            ops: self.ops,
            on_collect: self.on_collect.clone(),
        }
    }

    /// Create a read-only PatchSink that errors on collect.
    ///
    /// Used for `SealedState::get()` where writes are a programming error.
    #[doc(hidden)]
    pub fn read_only() -> Self {
        Self {
            ops: None,
            on_collect: None,
        }
    }

    /// Collect an operation.
    #[inline]
    pub fn collect(&self, op: Op) -> TireaResult<()> {
        let ops = self.ops.ok_or_else(|| {
            crate::TireaError::invalid_operation("write attempted on read-only state reference")
        })?;
        let mut guard = ops.lock().map_err(|_| {
            crate::TireaError::invalid_operation("state operation collector mutex poisoned")
        })?;
        guard.push(op.clone());
        drop(guard);
        if let Some(hook) = &self.on_collect {
            hook(&op)?;
        }
        Ok(())
    }

    /// Get the inner Mutex reference (for creating nested PatchSinks).
    #[doc(hidden)]
    pub fn inner(&self) -> &'a Mutex<Vec<Op>> {
        self.ops
            .expect("PatchSink::inner called on read-only sink (programming error)")
    }
}

/// Pure state context with automatic patch collection.
pub struct StateContext<'a> {
    doc: &'a DocCell,
    ops: Mutex<Vec<Op>>,
}

impl<'a> StateContext<'a> {
    /// Create a new pure state context.
    pub fn new(doc: &'a DocCell) -> Self {
        Self {
            doc,
            ops: Mutex::new(Vec::new()),
        }
    }

    /// Get a typed state reference at the specified path.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        let hook: CollectHook<'_> = Arc::new(|op: &Op| self.doc.apply(op));
        T::state_ref(self.doc, base, PatchSink::new_with_hook(&self.ops, hook))
    }

    /// Get a typed state reference at the type's canonical path.
    ///
    /// Requires `T` to have `#[tirea(path = "...")]` set.
    /// Panics if `T::PATH` is empty.
    pub fn state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use state::<T>(path) instead"
        );
        self.state::<T>(T::PATH)
    }

    /// Extract collected operations as a plain patch.
    pub fn take_patch(&self) -> Patch {
        let ops = std::mem::take(&mut *self.ops.lock().unwrap());
        Patch::with_ops(ops)
    }

    /// Extract collected operations as a tracked patch with a source.
    pub fn take_tracked_patch(&self, source: impl Into<String>) -> TrackedPatch {
        TrackedPatch::new(self.take_patch()).with_source(source)
    }

    /// Check if any operations have been collected.
    pub fn has_changes(&self) -> bool {
        !self.ops.lock().unwrap().is_empty()
    }

    /// Get the number of operations collected.
    pub fn ops_count(&self) -> usize {
        self.ops.lock().unwrap().len()
    }
}

/// Parse a dot-separated path string into a `Path`.
pub fn parse_path(path: &str) -> Path {
    if path.is_empty() {
        return Path::root();
    }

    let mut result = Path::root();
    for segment in path.split('.') {
        if !segment.is_empty() {
            result = result.key(segment);
        }
    }
    result
}

/// Trait for types that can create typed state references.
///
/// This trait is typically derived using `#[derive(State)]`.
/// It provides the interface for creating `StateRef` types that
/// allow typed read/write access to JSON documents.
///
/// # Example
///
/// ```ignore
/// use tirea_state::State;
/// use tirea_state_derive::State;
///
/// #[derive(State)]
/// struct User {
///     pub name: String,
///     pub age: i64,
/// }
///
/// // In a StateContext:
/// let user = ctx.state::<User>("users.alice");
/// let name = user.name()?;
/// user.set_name("Alice");
/// user.set_age(30);
/// ```
pub trait State: Sized {
    /// The reference type that provides typed access.
    type Ref<'a>;

    /// Canonical JSON path for this state type.
    ///
    /// When set via `#[tirea(path = "...")]`, enables `state_of::<T>()` access
    /// without an explicit path argument. Empty string means no bound path.
    const PATH: &'static str = "";

    /// Create a state reference at the specified path.
    ///
    /// # Arguments
    ///
    /// * `doc` - The JSON document to read from
    /// * `base` - The base path for this state
    /// * `sink` - The operation collector
    fn state_ref<'a>(doc: &'a DocCell, base: Path, sink: PatchSink<'a>) -> Self::Ref<'a>;

    /// Deserialize this type from a JSON value.
    fn from_value(value: &Value) -> TireaResult<Self>;

    /// Serialize this type to a JSON value.
    fn to_value(&self) -> TireaResult<Value>;

    /// Create a patch that sets this value at the root.
    fn to_patch(&self) -> TireaResult<Patch> {
        Ok(Patch::with_ops(vec![Op::set(
            Path::root(),
            self.to_value()?,
        )]))
    }
}

/// Extension trait providing convenience methods for State types.
pub trait StateExt: State {
    /// Create a state reference at the document root.
    fn at_root<'a>(doc: &'a DocCell, sink: PatchSink<'a>) -> Self::Ref<'a> {
        Self::state_ref(doc, Path::root(), sink)
    }
}

impl<T: State> StateExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_patch_sink_collect() {
        let ops = Mutex::new(Vec::new());
        let sink = PatchSink::new(&ops);

        sink.collect(Op::set(Path::root().key("a"), Value::from(1)))
            .unwrap();
        sink.collect(Op::set(Path::root().key("b"), Value::from(2)))
            .unwrap();

        let collected = ops.lock().unwrap();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_patch_sink_collect_hook() {
        let ops = Mutex::new(Vec::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_hook = seen.clone();
        let hook = Arc::new(move |op: &Op| {
            seen_hook.lock().unwrap().push(format!("{:?}", op));
            Ok(())
        });
        let sink = PatchSink::new_with_hook(&ops, hook);

        sink.collect(Op::set(Path::root().key("a"), Value::from(1)))
            .unwrap();
        sink.collect(Op::delete(Path::root().key("b"))).unwrap();

        let collected = ops.lock().unwrap();
        assert_eq!(collected.len(), 2);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn test_patch_sink_child_preserves_collect_and_hook() {
        let ops = Mutex::new(Vec::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_hook = seen.clone();
        let hook = Arc::new(move |op: &Op| {
            seen_hook.lock().unwrap().push(format!("{:?}", op));
            Ok(())
        });
        let sink = PatchSink::new_with_hook(&ops, hook);
        let child = sink.child();

        child
            .collect(Op::set(Path::root().key("nested"), Value::from(1)))
            .unwrap();

        assert_eq!(ops.lock().unwrap().len(), 1);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_patch_sink_read_only_child_collect_errors() {
        let sink = PatchSink::read_only();
        let child = sink.child();
        let err = child
            .collect(Op::set(Path::root().key("x"), Value::from(1)))
            .unwrap_err();
        assert!(matches!(err, crate::TireaError::InvalidOperation { .. }));
    }

    #[test]
    fn test_patch_sink_read_only_collect_errors() {
        let sink = PatchSink::read_only();
        let err = sink
            .collect(Op::set(Path::root().key("x"), Value::from(1)))
            .unwrap_err();
        assert!(matches!(err, crate::TireaError::InvalidOperation { .. }));
    }

    #[test]
    #[should_panic(expected = "read-only sink")]
    fn test_patch_sink_read_only_inner_panics() {
        let sink = PatchSink::read_only();
        let _ = sink.inner();
    }

    #[test]
    fn test_parse_path_empty() {
        let path = parse_path("");
        assert!(path.is_empty());
    }

    #[test]
    fn test_parse_path_nested() {
        let path = parse_path("tool_calls.call_123.data");
        assert_eq!(path.to_string(), "$.tool_calls.call_123.data");
    }

    #[test]
    fn test_state_context_collects_ops() {
        struct Counter;

        struct CounterRef<'a> {
            base: Path,
            sink: PatchSink<'a>,
        }

        impl<'a> CounterRef<'a> {
            fn set_value(&self, value: i64) -> TireaResult<()> {
                self.sink
                    .collect(Op::set(self.base.clone().key("value"), Value::from(value)))
            }
        }

        impl State for Counter {
            type Ref<'a> = CounterRef<'a>;

            fn state_ref<'a>(_: &'a DocCell, base: Path, sink: PatchSink<'a>) -> Self::Ref<'a> {
                CounterRef { base, sink }
            }

            fn from_value(_: &Value) -> TireaResult<Self> {
                Ok(Counter)
            }

            fn to_value(&self) -> TireaResult<Value> {
                Ok(Value::Null)
            }
        }

        let doc = DocCell::new(json!({"counter": {"value": 1}}));
        let ctx = StateContext::new(&doc);
        let counter = ctx.state::<Counter>("counter");
        counter.set_value(2).unwrap();

        assert!(ctx.has_changes());
        assert_eq!(ctx.ops_count(), 1);
        assert_eq!(ctx.take_patch().len(), 1);
    }
}
