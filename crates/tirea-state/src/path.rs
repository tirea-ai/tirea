//! JSON path representation for navigating document structure.
//!
//! Paths are sequences of segments that describe a location in a JSON document.
//! Each segment is either a key (for objects) or an index (for arrays).

use serde::{Deserialize, Serialize};
use std::fmt;

/// A single segment in a JSON path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Seg {
    /// Object key access: `{"key": value}`
    Key(String),
    /// Array index access: `[index]`
    Index(usize),
}

impl Seg {
    /// Create a key segment.
    #[inline]
    pub fn key(k: impl Into<String>) -> Self {
        Seg::Key(k.into())
    }

    /// Create an index segment.
    #[inline]
    pub fn index(i: usize) -> Self {
        Seg::Index(i)
    }

    /// Returns true if this is a key segment.
    #[inline]
    pub fn is_key(&self) -> bool {
        matches!(self, Seg::Key(_))
    }

    /// Returns true if this is an index segment.
    #[inline]
    pub fn is_index(&self) -> bool {
        matches!(self, Seg::Index(_))
    }

    /// Get the key if this is a key segment.
    #[inline]
    pub fn as_key(&self) -> Option<&str> {
        match self {
            Seg::Key(k) => Some(k),
            Seg::Index(_) => None,
        }
    }

    /// Get the index if this is an index segment.
    #[inline]
    pub fn as_index(&self) -> Option<usize> {
        match self {
            Seg::Key(_) => None,
            Seg::Index(i) => Some(*i),
        }
    }
}

impl fmt::Display for Seg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Seg::Key(k) => write!(f, ".{}", k),
            Seg::Index(i) => write!(f, "[{}]", i),
        }
    }
}

impl From<String> for Seg {
    fn from(s: String) -> Self {
        Seg::Key(s)
    }
}

impl From<&str> for Seg {
    fn from(s: &str) -> Self {
        Seg::Key(s.to_owned())
    }
}

impl From<usize> for Seg {
    fn from(i: usize) -> Self {
        Seg::Index(i)
    }
}

/// A complete path into a JSON structure.
///
/// Paths are immutable sequences of segments. Use builder methods to construct
/// paths incrementally.
///
/// # Examples
///
/// ```
/// use tirea_state::Path;
///
/// let path = Path::root().key("users").index(0).key("name");
/// assert_eq!(path.len(), 3);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Path(Vec<Seg>);

impl Path {
    /// Create an empty path (root).
    #[inline]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Create an empty path (alias for `new`).
    #[inline]
    pub fn root() -> Self {
        Self::new()
    }

    /// Create a path from a vector of segments.
    #[inline]
    pub fn from_segments(segments: Vec<Seg>) -> Self {
        Self(segments)
    }

    /// Create a path with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Append a key segment and return self (builder pattern).
    #[inline]
    pub fn key(mut self, k: impl Into<String>) -> Self {
        self.0.push(Seg::Key(k.into()));
        self
    }

    /// Append an index segment and return self (builder pattern).
    #[inline]
    pub fn index(mut self, i: usize) -> Self {
        self.0.push(Seg::Index(i));
        self
    }

    /// Push a segment onto the path (mutating).
    #[inline]
    pub fn push(&mut self, seg: Seg) {
        self.0.push(seg);
    }

    /// Push a key segment onto the path (mutating).
    #[inline]
    pub fn push_key(&mut self, k: impl Into<String>) {
        self.0.push(Seg::Key(k.into()));
    }

    /// Push an index segment onto the path (mutating).
    #[inline]
    pub fn push_index(&mut self, i: usize) {
        self.0.push(Seg::Index(i));
    }

    /// Pop the last segment from the path.
    #[inline]
    pub fn pop(&mut self) -> Option<Seg> {
        self.0.pop()
    }

    /// Get the segments of this path.
    #[inline]
    pub fn segments(&self) -> &[Seg] {
        &self.0
    }

    /// Get a mutable reference to the segments.
    #[inline]
    pub fn segments_mut(&mut self) -> &mut Vec<Seg> {
        &mut self.0
    }

    /// Check if this path is empty (root).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get the number of segments in this path.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Get the first segment.
    #[inline]
    pub fn first(&self) -> Option<&Seg> {
        self.0.first()
    }

    /// Get the last segment.
    #[inline]
    pub fn last(&self) -> Option<&Seg> {
        self.0.last()
    }

    /// Join this path with another path.
    #[inline]
    pub fn join(&self, other: &Path) -> Path {
        let mut result = self.clone();
        result.0.extend(other.0.iter().cloned());
        result
    }

    /// Append a segment and return a new path (non-mutating builder).
    #[inline]
    pub fn with_segment(&self, seg: Seg) -> Path {
        let mut result = self.clone();
        result.0.push(seg);
        result
    }

    /// Check if this path is a prefix of another path.
    ///
    /// A path is a prefix of another if all of its segments match
    /// the beginning of the other path's segments.
    ///
    /// # Examples
    ///
    /// ```
    /// use tirea_state::path;
    ///
    /// let parent = path!("user");
    /// let child = path!("user", "name");
    ///
    /// assert!(parent.is_prefix_of(&child));
    /// assert!(!child.is_prefix_of(&parent));
    /// assert!(parent.is_prefix_of(&parent)); // A path is a prefix of itself
    /// ```
    #[inline]
    pub fn is_prefix_of(&self, other: &Path) -> bool {
        if self.len() > other.len() {
            return false;
        }
        self.0.iter().zip(other.0.iter()).all(|(a, b)| a == b)
    }

    /// Extend this path with segments from another path.
    #[inline]
    pub fn extend(&mut self, other: &Path) {
        self.0.extend(other.0.iter().cloned());
    }

    /// Get the parent path (path without the last segment).
    #[inline]
    pub fn parent(&self) -> Option<Path> {
        if self.0.is_empty() {
            None
        } else {
            let mut p = self.clone();
            p.pop();
            Some(p)
        }
    }

    /// Check if this path starts with another path.
    #[inline]
    pub fn starts_with(&self, prefix: &Path) -> bool {
        self.0.starts_with(&prefix.0)
    }

    /// Get a slice of segments from start to end.
    #[inline]
    pub fn slice(&self, start: usize, end: usize) -> Path {
        Path(self.0[start..end].to_vec())
    }

    /// Iterate over the segments.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &Seg> {
        self.0.iter()
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "$")
        } else {
            write!(f, "$")?;
            for seg in &self.0 {
                write!(f, "{}", seg)?;
            }
            Ok(())
        }
    }
}

impl FromIterator<Seg> for Path {
    fn from_iter<I: IntoIterator<Item = Seg>>(iter: I) -> Self {
        Path(iter.into_iter().collect())
    }
}

impl IntoIterator for Path {
    type Item = Seg;
    type IntoIter = std::vec::IntoIter<Seg>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Path {
    type Item = &'a Seg;
    type IntoIter = std::slice::Iter<'a, Seg>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl std::ops::Index<usize> for Path {
    type Output = Seg;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

/// Construct a `Path` from a sequence of segments.
///
/// # Examples
///
/// ```
/// use tirea_state::path;
///
/// // String literals become Key segments
/// let p = path!("users", "alice", "email");
///
/// // Numbers become Index segments
/// let p = path!("items", 0, "name");
///
/// // Mixed types
/// let p = path!("data", "list", 2, "value");
/// ```
#[macro_export]
macro_rules! path {
    () => {
        $crate::Path::root()
    };
    ($($seg:expr),+ $(,)?) => {{
        let mut p = $crate::Path::root();
        $(
            p.push($crate::path!(@seg $seg));
        )+
        p
    }};
    (@seg $seg:expr) => {
        $crate::Seg::from($seg)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_construction() {
        let path = Path::root().key("users").index(0).key("name");
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], Seg::Key("users".into()));
        assert_eq!(path[1], Seg::Index(0));
        assert_eq!(path[2], Seg::Key("name".into()));
    }

    #[test]
    fn test_path_display() {
        let path = Path::root().key("users").index(0).key("name");
        assert_eq!(format!("{}", path), "$.users[0].name");
    }

    #[test]
    fn test_path_macro() {
        let p = path!("users", 0, "name");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], Seg::Key("users".into()));
        assert_eq!(p[1], Seg::Index(0));
        assert_eq!(p[2], Seg::Key("name".into()));
    }

    #[test]
    fn test_path_join() {
        let base = Path::root().key("data");
        let sub = Path::root().key("items").index(0);
        let joined = base.join(&sub);
        assert_eq!(joined.len(), 3);
    }

    #[test]
    fn test_path_parent() {
        let path = Path::root().key("a").key("b");
        let parent = path.parent().unwrap();
        assert_eq!(parent.len(), 1);
        assert_eq!(parent[0], Seg::Key("a".into()));
    }

    #[test]
    fn test_path_serde() {
        let path = Path::root().key("users").index(0);
        let json = serde_json::to_string(&path).unwrap();
        let parsed: Path = serde_json::from_str(&json).unwrap();
        assert_eq!(path, parsed);
    }
}
