//! State trait for typed state access.
//!
//! The `State` trait provides a unified interface for typed access to JSON documents.
//! It is typically implemented via the derive macro `#[derive(State)]`.

use crate::{CarveResult, Op, Patch, Path};
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
    ops: &'a Mutex<Vec<Op>>,
    on_collect: Option<CollectHook<'a>>,
}

impl<'a> PatchSink<'a> {
    /// Create a new PatchSink wrapping a Mutex.
    #[doc(hidden)]
    pub fn new(ops: &'a Mutex<Vec<Op>>) -> Self {
        Self {
            ops,
            on_collect: None,
        }
    }

    /// Create a new PatchSink with a collect hook.
    ///
    /// The hook is invoked after each operation is collected.
    #[doc(hidden)]
    pub fn new_with_hook(ops: &'a Mutex<Vec<Op>>, hook: CollectHook<'a>) -> Self {
        Self {
            ops,
            on_collect: Some(hook),
        }
    }

    /// Collect an operation.
    #[inline]
    pub fn collect(&self, op: Op) {
        self.ops.lock().unwrap().push(op.clone());
        if let Some(hook) = &self.on_collect {
            hook(&op);
        }
    }

    /// Get the inner Mutex reference (for creating nested PatchSinks).
    #[doc(hidden)]
    pub fn inner(&self) -> &'a Mutex<Vec<Op>> {
        self.ops
    }
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
/// // In a Context:
/// let user = ctx.state::<User>("users.alice");
/// let name = user.name()?;
/// user.set_name("Alice");
/// user.set_age(30);
/// ```
pub trait State: Sized {
    /// The reference type that provides typed access.
    type Ref<'a>;

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
}
