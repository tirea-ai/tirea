/// A collection with cursor-based delta tracking.
///
/// `DeltaTracked<T>` wraps a `Vec<T>` and tracks which items have been
/// consumed via `take_delta()`. This provides an efficient way to accumulate
/// items over time and periodically extract only the new ones.
///
/// The cursor marks the position up to which items have been taken.
/// `take_delta()` returns `items[cursor..]` and advances the cursor.
#[derive(Debug, Clone)]
pub struct DeltaTracked<T> {
    items: Vec<T>,
    cursor: usize,
}

impl<T> DeltaTracked<T> {
    /// Create from existing items with cursor at the end (no pending delta).
    pub fn new(items: Vec<T>) -> Self {
        let cursor = items.len();
        Self { items, cursor }
    }

    /// Create an empty tracker with cursor at 0.
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            cursor: 0,
        }
    }

    /// Append a single item.
    pub fn push(&mut self, item: T) {
        self.items.push(item);
    }

    /// Append multiple items.
    pub fn extend(&mut self, iter: impl IntoIterator<Item = T>) {
        self.items.extend(iter);
    }

    /// View all items (including already-consumed ones).
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    /// Total number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Consume and return the inner `Vec<T>`.
    pub fn into_items(self) -> Vec<T> {
        self.items
    }

    /// Whether there are items after the cursor.
    pub fn has_delta(&self) -> bool {
        self.cursor < self.items.len()
    }
}

impl<T: Clone> DeltaTracked<T> {
    /// Clone and return items added since the last `take_delta()`, then advance the cursor.
    pub fn take_delta(&mut self) -> Vec<T> {
        let delta = self.items[self.cursor..].to_vec();
        self.cursor = self.items.len();
        delta
    }
}

impl<T> Default for DeltaTracked<T> {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_no_delta() {
        let mut dt = DeltaTracked::new(vec![1, 2, 3]);
        assert!(!dt.has_delta());
        assert_eq!(dt.take_delta(), Vec::<i32>::new());
        assert_eq!(dt.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn empty_has_no_delta() {
        let dt = DeltaTracked::<i32>::empty();
        assert!(!dt.has_delta());
        assert!(dt.is_empty());
    }

    #[test]
    fn push_creates_delta() {
        let mut dt = DeltaTracked::new(vec![1, 2]);
        dt.push(3);
        dt.push(4);
        assert!(dt.has_delta());
        assert_eq!(dt.take_delta(), vec![3, 4]);
        assert!(!dt.has_delta());
        assert_eq!(dt.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn extend_creates_delta() {
        let mut dt = DeltaTracked::new(vec![1]);
        dt.extend(vec![2, 3]);
        assert_eq!(dt.take_delta(), vec![2, 3]);
    }

    #[test]
    fn empty_then_push() {
        let mut dt = DeltaTracked::empty();
        dt.push(1);
        dt.push(2);
        assert!(dt.has_delta());
        assert_eq!(dt.take_delta(), vec![1, 2]);
        assert!(!dt.has_delta());
    }

    #[test]
    fn multiple_takes() {
        let mut dt = DeltaTracked::empty();
        dt.push(1);
        assert_eq!(dt.take_delta(), vec![1]);
        dt.push(2);
        dt.push(3);
        assert_eq!(dt.take_delta(), vec![2, 3]);
        assert_eq!(dt.take_delta(), Vec::<i32>::new());
    }

    #[test]
    fn into_items() {
        let mut dt = DeltaTracked::new(vec![1, 2]);
        dt.push(3);
        assert_eq!(dt.into_items(), vec![1, 2, 3]);
    }

    #[test]
    fn len_and_is_empty() {
        let mut dt = DeltaTracked::empty();
        assert!(dt.is_empty());
        assert_eq!(dt.len(), 0);
        dt.push(1);
        assert!(!dt.is_empty());
        assert_eq!(dt.len(), 1);
    }

    #[test]
    fn default_is_empty() {
        let dt = DeltaTracked::<String>::default();
        assert!(dt.is_empty());
        assert!(!dt.has_delta());
    }
}
