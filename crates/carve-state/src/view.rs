//! CarveViewModel trait for typed views.
//!
//! This trait is typically implemented via the derive macro `#[derive(CarveViewModel)]`.
//! It provides the interface for creating strongly-typed readers, writers, and accessors.

use crate::{AccessorOps, CarveResult, Patch, Path, WriterOps};
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
/// - `Accessor<'a>`: A unified accessor that combines read and write capabilities
///
/// # Examples
///
/// ```
/// use carve_state::{CarveViewModel, CarveViewModelExt};
/// use carve_state_derive::CarveViewModel;
/// use serde::{Serialize, Deserialize};
/// use serde_json::json;
///
/// #[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
/// struct User {
///     name: String,
///     age: u32,
/// }
///
/// // Read from document
/// let doc = json!({"name": "Alice", "age": 30});
/// let reader = User::read(&doc);
/// assert_eq!(reader.name().unwrap(), "Alice");
///
/// // Write changes
/// let mut writer = User::write();
/// writer.name("Bob");
/// writer.age(25);
/// let patch = writer.build();
/// assert_eq!(patch.len(), 2);
/// ```
pub trait CarveViewModel: Sized {
    /// Strongly-typed reader for this view model.
    type Reader<'a>
    where
        Self: 'a;

    /// Strongly-typed writer for this view model.
    type Writer: WriterOps;

    /// Unified accessor combining read and write capabilities.
    ///
    /// The Accessor type provides field-like access with operator support
    /// for numeric types and collection methods for Vec/Map types.
    type Accessor<'a>: AccessorOps
    where
        Self: 'a;

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

    /// Create an accessor at the specified base path.
    ///
    /// This is typically used by frameworks that manage path routing.
    /// For simple cases, use `T::access(&doc)` instead.
    #[doc(hidden)]
    fn accessor(doc: &Value, base: Path) -> Self::Accessor<'_>;

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

    /// Create an accessor at the document root.
    ///
    /// This is the primary way to create an accessor when you don't need
    /// to specify a base path. The accessor combines read and write capabilities.
    fn access(doc: &Value) -> Self::Accessor<'_> {
        Self::accessor(doc, Path::root())
    }
}

// Blanket implementation for all CarveViewModel types
impl<T: CarveViewModel> CarveViewModelExt for T {}

#[cfg(test)]
mod tests {
    // Tests will be in integration tests once the derive macro is implemented
}
