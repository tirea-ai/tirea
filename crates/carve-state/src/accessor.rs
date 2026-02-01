//! AccessorOps trait for unified read/write access.
//!
//! This trait defines the common interface for Accessor types,
//! which combine the functionality of Reader and Writer into a single type.

use crate::Patch;

/// Common operations for Accessor types.
///
/// An Accessor combines read and write capabilities into a single type.
/// It accumulates operations internally and can produce a Patch when done.
///
/// # Example
///
/// ```ignore
/// let mut accessor = state.access::<Counter>();
/// let current = accessor.counter()?;
/// accessor.set_counter(current + 1);
/// state.commit(accessor)?;  // Commits using AccessorOps::build()
/// ```
pub trait AccessorOps {
    /// Check if there are any pending operations.
    fn has_changes(&self) -> bool;

    /// Get the number of pending operations.
    fn len(&self) -> usize;

    /// Check if there are no pending operations.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Consume the accessor and build a Patch from accumulated operations.
    fn build(self) -> Patch;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Op, Path};
    use std::cell::RefCell;

    /// A simple test accessor to verify the trait.
    struct TestAccessor {
        ops: RefCell<Vec<Op>>,
    }

    impl TestAccessor {
        fn new() -> Self {
            Self {
                ops: RefCell::new(Vec::new()),
            }
        }

        fn set_value(&self, value: i32) {
            self.ops
                .borrow_mut()
                .push(Op::set(Path::root().key("value"), serde_json::Value::from(value)));
        }
    }

    impl AccessorOps for TestAccessor {
        fn has_changes(&self) -> bool {
            !self.ops.borrow().is_empty()
        }

        fn len(&self) -> usize {
            self.ops.borrow().len()
        }

        fn build(self) -> Patch {
            Patch::with_ops(self.ops.into_inner())
        }
    }

    #[test]
    fn test_accessor_ops() {
        let accessor = TestAccessor::new();
        assert!(!accessor.has_changes());
        assert!(accessor.is_empty());

        accessor.set_value(42);
        assert!(accessor.has_changes());
        assert_eq!(accessor.len(), 1);

        let patch = accessor.build();
        assert_eq!(patch.len(), 1);
    }
}
