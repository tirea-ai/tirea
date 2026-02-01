//! State container for unified state management.
//!
//! The `State` type provides a unified container for JSON state with typed view access.

use crate::{apply_patch, CarveResult, CarveViewModel, CarveViewModelExt, Path, Patch, Op};
use serde_json::Value;

/// A unified state container wrapping a JSON document.
///
/// `State` provides multiple ways to access and modify the underlying JSON:
/// - Raw access via `raw()` and `snapshot()`
/// - Path-based access via `get()`, `set()`, `delete()`
/// - Typed view access via `read()` and `write()`
/// - Patch application via `apply()`
///
/// # Examples
///
/// ```
/// use carve_state::{State, CarveViewModel, CarveViewModelExt};
/// use carve_state_derive::CarveViewModel;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
/// struct Counter {
///     value: i32,
/// }
///
/// let mut state = State::new();
///
/// // Write using typed writer
/// let mut w = state.write::<Counter>();
/// w.value(42);
/// state.apply(&w.build()).unwrap();
///
/// // Read using typed reader
/// let r = state.read::<Counter>();
/// assert_eq!(r.value().unwrap(), 42);
/// ```
#[derive(Clone, Debug, Default)]
pub struct State {
    doc: Value,
}

impl State {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            doc: Value::Object(Default::default()),
        }
    }

    /// Create a state from an existing JSON value.
    pub fn from_value(value: Value) -> Self {
        Self { doc: value }
    }

    /// Create a state from a serializable type.
    pub fn from_model<T: serde::Serialize>(model: &T) -> Self {
        Self {
            doc: serde_json::to_value(model).unwrap_or(Value::Object(Default::default())),
        }
    }

    // ===== Raw Access =====

    /// Get a reference to the underlying JSON document.
    pub fn raw(&self) -> &Value {
        &self.doc
    }

    /// Get a clone of the underlying JSON document.
    pub fn snapshot(&self) -> Value {
        self.doc.clone()
    }

    // ===== Path-based Access =====

    /// Get a value at the specified path.
    pub fn get(&self, path: &Path) -> Option<&Value> {
        crate::get_at_path(&self.doc, path)
    }

    /// Set a value at the specified path.
    pub fn set(&mut self, path: Path, value: impl Into<Value>) -> CarveResult<()> {
        let patch = Patch::new().with_op(Op::set(path, value.into()));
        self.apply(&patch)
    }

    /// Delete a value at the specified path.
    pub fn delete(&mut self, path: Path) -> CarveResult<()> {
        let patch = Patch::new().with_op(Op::delete(path));
        self.apply(&patch)
    }

    // ===== Typed View Access =====

    /// Create a typed reader for this state.
    ///
    /// The reader provides strongly-typed access to fields defined in the view model.
    pub fn read<V: CarveViewModel>(&self) -> V::Reader<'_> {
        V::read(&self.doc)
    }

    /// Create a typed writer for this state.
    ///
    /// The writer accumulates operations that can be applied via `apply()`.
    pub fn write<V: CarveViewModel>(&self) -> V::Writer {
        V::write()
    }

    /// Create a typed accessor for this state.
    ///
    /// The accessor combines read and write capabilities, providing
    /// field proxy types with operator support for numeric types.
    ///
    /// # Usage Pattern
    ///
    /// Due to Rust's borrow checking, the accessor holds a reference to the
    /// state's document. To apply changes, use the `build()` method to get
    /// a patch, then apply it:
    ///
    /// ```ignore
    /// let patch = {
    ///     let accessor = state.access::<Counter>();
    ///     let current = accessor.value().get()?;
    ///     accessor.set_value(current + 1);
    ///     accessor.build() // Consumes accessor, releasing borrow
    /// };
    /// state.apply(&patch)?;
    /// ```
    pub fn access<V: CarveViewModel>(&self) -> V::Accessor<'_> {
        V::access(&self.doc)
    }


    // ===== Patch Operations =====

    /// Apply a patch to this state.
    pub fn apply(&mut self, patch: &Patch) -> CarveResult<()> {
        if !patch.is_empty() {
            self.doc = apply_patch(&self.doc, patch)?;
        }
        Ok(())
    }

    /// Merge another state into this one (shallow merge of top-level keys).
    pub fn merge(&mut self, other: &State) -> CarveResult<()> {
        if let (Value::Object(a), Value::Object(b)) = (&mut self.doc, &other.doc) {
            for (k, v) in b {
                a.insert(k.clone(), v.clone());
            }
        }
        Ok(())
    }

    // ===== Serialization =====

    /// Serialize the state to a JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.doc).unwrap_or_default()
    }

    /// Serialize the state to a pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(&self.doc).unwrap_or_default()
    }

    /// Deserialize a state from a JSON string.
    pub fn from_json(json: &str) -> CarveResult<Self> {
        let doc = serde_json::from_str(json)?;
        Ok(Self { doc })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path;

    #[test]
    fn test_new_state() {
        let state = State::new();
        assert_eq!(state.raw(), &Value::Object(Default::default()));
    }

    #[test]
    fn test_from_value() {
        let value = serde_json::json!({"counter": 42});
        let state = State::from_value(value.clone());
        assert_eq!(state.raw(), &value);
    }

    #[test]
    fn test_path_access() {
        let mut state = State::new();

        // Set
        state.set(path!("counter"), 42).unwrap();
        assert_eq!(state.get(&path!("counter")), Some(&serde_json::json!(42)));

        // Delete
        state.delete(path!("counter")).unwrap();
        assert_eq!(state.get(&path!("counter")), None);
    }

    #[test]
    fn test_serialization() {
        let mut state = State::new();
        state.set(path!("name"), "Alice").unwrap();

        let json = state.to_json();
        let restored = State::from_json(&json).unwrap();

        assert_eq!(state.raw(), restored.raw());
    }
}
