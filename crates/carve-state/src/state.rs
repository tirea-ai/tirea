//! State trait for typed state access.
//!
//! The `State` trait provides a unified interface for typed access to JSON documents.
//! It is typically implemented via the derive macro `#[derive(State)]`.

use crate::{CarveResult, Op, Patch, Path, TrackedPatch};
use serde_json::Value;
use std::sync::{Arc, Mutex};

type CollectHook<'a> = Arc<dyn Fn(&Op) + Send + Sync + 'a>;

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

    /// Create a read-only PatchSink that panics on collect.
    ///
    /// Used for `ScopeState::get()` where writes are a programming error.
    #[doc(hidden)]
    pub fn read_only() -> Self {
        Self {
            ops: None,
            on_collect: None,
        }
    }

    /// Collect an operation.
    #[inline]
    pub fn collect(&self, op: Op) {
        let ops = self
            .ops
            .expect("PatchSink::collect called on read-only sink (programming error)");
        ops.lock().unwrap().push(op.clone());
        if let Some(hook) = &self.on_collect {
            hook(&op);
        }
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
    doc: &'a Value,
    ops: Mutex<Vec<Op>>,
    run_overlay: Option<&'a Mutex<Vec<Op>>>,
}

impl<'a> StateContext<'a> {
    /// Create a new pure state context.
    pub fn new(doc: &'a Value) -> Self {
        Self {
            doc,
            ops: Mutex::new(Vec::new()),
            run_overlay: None,
        }
    }

    /// Create a state context with a run overlay for override writes.
    pub fn with_overlay(doc: &'a Value, overlay: &'a Mutex<Vec<Op>>) -> Self {
        Self {
            doc,
            ops: Mutex::new(Vec::new()),
            run_overlay: Some(overlay),
        }
    }

    /// Get a typed state reference at the specified path.
    pub fn state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let base = parse_path(path);
        T::state_ref(self.doc, base, PatchSink::new(&self.ops))
    }

    /// Get a typed state reference at the type's canonical path.
    ///
    /// Requires `T` to have `#[carve(path = "...")]` set.
    /// Panics if `T::PATH` is empty.
    pub fn state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use state::<T>(path) instead"
        );
        self.state::<T>(T::PATH)
    }

    /// Typed state reference that writes to the run overlay (not persisted).
    ///
    /// Panics if this context was not created with `with_overlay()`.
    pub fn override_state<T: State>(&self, path: &str) -> T::Ref<'_> {
        let overlay = self
            .run_overlay
            .expect("override_state called on StateContext without overlay");
        T::state_ref(self.doc, parse_path(path), PatchSink::new(overlay))
    }

    /// Typed state reference at canonical path, writing to the run overlay.
    ///
    /// Panics if `T::PATH` is empty or if no overlay is set.
    pub fn override_state_of<T: State>(&self) -> T::Ref<'_> {
        assert!(
            !T::PATH.is_empty(),
            "State type has no bound path; use override_state::<T>(path) instead"
        );
        self.override_state::<T>(T::PATH)
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
/// use carve_state::State;
/// use carve_state_derive::State;
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
    /// When set via `#[carve(path = "...")]`, enables `state_of::<T>()` access
    /// without an explicit path argument. Empty string means no bound path.
    const PATH: &'static str = "";

    /// Create a state reference at the specified path.
    ///
    /// # Arguments
    ///
    /// * `doc` - The JSON document to read from
    /// * `base` - The base path for this state
    /// * `sink` - The operation collector
    fn state_ref<'a>(doc: &'a Value, base: Path, sink: PatchSink<'a>) -> Self::Ref<'a>;

    /// Deserialize this type from a JSON value.
    fn from_value(value: &Value) -> CarveResult<Self>;

    /// Serialize this type to a JSON value.
    fn to_value(&self) -> Value;

    /// Create a patch that sets this value at the root.
    fn to_patch(&self) -> Patch {
        Patch::with_ops(vec![Op::set(Path::root(), self.to_value())])
    }
}

/// Extension trait providing convenience methods for State types.
pub trait StateExt: State {
    /// Create a state reference at the document root.
    fn at_root<'a>(doc: &'a Value, sink: PatchSink<'a>) -> Self::Ref<'a> {
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

        sink.collect(Op::set(Path::root().key("a"), Value::from(1)));
        sink.collect(Op::set(Path::root().key("b"), Value::from(2)));

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
        });
        let sink = PatchSink::new_with_hook(&ops, hook);

        sink.collect(Op::set(Path::root().key("a"), Value::from(1)));
        sink.collect(Op::delete(Path::root().key("b")));

        let collected = ops.lock().unwrap();
        assert_eq!(collected.len(), 2);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    #[should_panic(expected = "read-only sink")]
    fn test_patch_sink_read_only_collect_panics() {
        let sink = PatchSink::read_only();
        sink.collect(Op::set(Path::root().key("x"), Value::from(1)));
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
            fn set_value(&self, value: i64) {
                self.sink
                    .collect(Op::set(self.base.clone().key("value"), Value::from(value)));
            }
        }

        impl State for Counter {
            type Ref<'a> = CounterRef<'a>;

            fn state_ref<'a>(_: &'a Value, base: Path, sink: PatchSink<'a>) -> Self::Ref<'a> {
                CounterRef { base, sink }
            }

            fn from_value(_: &Value) -> CarveResult<Self> {
                Ok(Counter)
            }

            fn to_value(&self) -> Value {
                Value::Null
            }
        }

        let doc = json!({"counter": {"value": 1}});
        let ctx = StateContext::new(&doc);
        let counter = ctx.state::<Counter>("counter");
        counter.set_value(2);

        assert!(ctx.has_changes());
        assert_eq!(ctx.ops_count(), 1);
        assert_eq!(ctx.take_patch().len(), 1);
    }
}
