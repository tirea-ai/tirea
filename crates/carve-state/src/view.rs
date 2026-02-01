//! CarveViewModel trait for typed views.
//!
//! This trait is typically implemented via the derive macro `#[derive(CarveViewModel)]`.
//! It provides the interface for creating strongly-typed readers and writers.

use crate::{CarveResult, Patch, Path};
use serde_json::Value;

/// A typed view model for a portion of a JSON document.
///
/// This trait provides the interface between typed Rust structs and JSON documents.
/// It is typically derived using `#[derive(CarveViewModel)]`.
///
/// # Associated Types
///
/// - `Reader<'a>`: A strongly-typed reader that provides field accessors
/// - `Writer`: A strongly-typed writer that provides field setters
///
/// # Examples
///
/// ```ignore
/// use carve_state::CarveViewModel;
///
/// #[derive(CarveViewModel)]
/// struct User {
///     name: String,
///     age: u32,
/// }
///
/// // Read from document
/// let doc = json!({"name": "Alice", "age": 30});
/// let reader = User::read(&doc);
/// assert_eq!(reader.name()?, "Alice");
///
/// // Write changes
/// let mut writer = User::write();
/// writer.name("Bob");
/// writer.age(25);
/// let patch = writer.build();
/// ```
pub trait CarveViewModel: Sized {
    /// Strongly-typed reader for this view model.
    type Reader<'a>
    where
        Self: 'a;

    /// Strongly-typed writer for this view model.
    type Writer;

    /// Create a reader at the specified base path.
    ///
    /// This is typically used by frameworks that manage path routing.
    /// For simple cases, use `T::read(&doc)` instead.
    #[doc(hidden)]
    fn reader(doc: &Value, base: Path) -> Self::Reader<'_>;

    /// Create a writer at the specified base path.
    ///
    /// This is typically used by frameworks that manage path routing.
    /// For simple cases, use `T::write()` instead.
    #[doc(hidden)]
    fn writer(base: Path) -> Self::Writer;

    /// Deserialize this type from a JSON value.
    fn from_value(value: &Value) -> CarveResult<Self>;

    /// Serialize this type to a JSON value.
    fn to_value(&self) -> Value;

    /// Create a patch that sets this value at the root.
    fn to_patch(&self) -> Patch {
        Patch::with_ops(vec![crate::Op::set(Path::root(), self.to_value())])
    }
}

/// Extension trait providing convenience methods for CarveViewModel types.
///
/// These methods are automatically available on any type that implements
/// `CarveViewModel`. They hide the Path parameter for simple use cases.
pub trait CarveViewModelExt: CarveViewModel {
    /// Create a reader at the document root.
    ///
    /// This is the primary way to read from a document when you don't need
    /// to specify a base path.
    fn read(doc: &Value) -> Self::Reader<'_> {
        Self::reader(doc, Path::root())
    }

    /// Create a writer at the document root.
    ///
    /// This is the primary way to create a writer when you don't need
    /// to specify a base path.
    fn write() -> Self::Writer {
        Self::writer(Path::root())
    }
}

// Blanket implementation for all CarveViewModel types
impl<T: CarveViewModel> CarveViewModelExt for T {}

#[cfg(test)]
mod tests {
    // Tests will be in integration tests once the derive macro is implemented
}
