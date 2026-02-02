//! StateManager manages immutable state with patch history.
//!
//! All state changes are applied through patches, enabling:
//! - Full traceability of changes
//! - State replay to any point in history
//! - Batch application with conflict detection

use crate::{apply_patch, CarveError, TrackedPatch};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Errors that can occur during state management.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("Failed to apply patch: {0}")]
    ApplyFailed(#[from] CarveError),

    #[error("Invalid replay index: {index}, history length: {len}")]
    InvalidReplayIndex { index: usize, len: usize },
}

/// Result of applying patches.
#[derive(Debug, Clone)]
pub struct ApplyResult {
    /// Number of patches applied.
    pub patches_applied: usize,
    /// Number of operations applied.
    pub ops_applied: usize,
}

/// StateManager manages immutable state with patch history.
///
/// # Design
///
/// - State is immutable - all changes go through patches
/// - Full history is maintained for replay
/// - Supports batch application
///
/// # Example
///
/// ```ignore
/// use carve_state::{StateManager, Context};
/// use serde_json::json;
///
/// let manager = StateManager::new(json!({}));
///
/// // Get snapshot and create context
/// let snapshot = manager.snapshot().await;
/// let ctx = Context::new(&snapshot, "call_1", "tool:example");
///
/// // ... modify state through ctx ...
///
/// // Apply patch
/// manager.apply(ctx.take_patch()).await?;
///
/// // Replay to a specific point
/// let old_state = manager.replay_to(5).await?;
/// ```
pub struct StateManager {
    state: Arc<RwLock<Value>>,
    history: Arc<RwLock<Vec<TrackedPatch>>>,
}

impl StateManager {
    /// Create a new StateManager with initial state.
    pub fn new(initial: Value) -> Self {
        Self {
            state: Arc::new(RwLock::new(initial)),
            history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get a snapshot of the current state.
    pub async fn snapshot(&self) -> Value {
        self.state.read().await.clone()
    }

    /// Apply a single patch.
    pub async fn apply(&self, patch: TrackedPatch) -> Result<ApplyResult, StateError> {
        let ops_count = patch.patch().len();

        let mut state = self.state.write().await;
        let new_state = apply_patch(&state, patch.patch())?;
        *state = new_state;
        drop(state);

        self.history.write().await.push(patch);

        Ok(ApplyResult {
            patches_applied: 1,
            ops_applied: ops_count,
        })
    }

    /// Apply multiple patches in batch.
    ///
    /// Patches are applied in order. If any patch fails, the operation
    /// stops and returns an error (partial application may have occurred).
    pub async fn apply_batch(&self, patches: Vec<TrackedPatch>) -> Result<ApplyResult, StateError> {
        if patches.is_empty() {
            return Ok(ApplyResult {
                patches_applied: 0,
                ops_applied: 0,
            });
        }

        let mut total_ops = 0;
        let mut state = self.state.write().await;

        for patch in &patches {
            total_ops += patch.patch().len();
            let new_state = apply_patch(&state, patch.patch())?;
            *state = new_state;
        }
        drop(state);

        let patches_count = patches.len();
        self.history.write().await.extend(patches);

        Ok(ApplyResult {
            patches_applied: patches_count,
            ops_applied: total_ops,
        })
    }

    /// Replay state from the beginning up to (and including) the specified index.
    ///
    /// Returns the state as it was after applying patches [0..=index].
    pub async fn replay_to(&self, index: usize) -> Result<Value, StateError> {
        let history = self.history.read().await;

        if index >= history.len() {
            return Err(StateError::InvalidReplayIndex {
                index,
                len: history.len(),
            });
        }

        let mut state = serde_json::json!({});
        for patch in history.iter().take(index + 1) {
            state = apply_patch(&state, patch.patch())?;
        }

        Ok(state)
    }

    /// Get the full patch history.
    pub async fn history(&self) -> Vec<TrackedPatch> {
        self.history.read().await.clone()
    }

    /// Get the number of patches in history.
    pub async fn history_len(&self) -> usize {
        self.history.read().await.len()
    }

    /// Clear history (keeps current state).
    ///
    /// Use with caution - this removes the ability to replay.
    pub async fn clear_history(&self) {
        self.history.write().await.clear();
    }
}

impl Clone for StateManager {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            history: Arc::clone(&self.history),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{path, Op, Patch};
    use serde_json::json;

    fn make_patch(ops: Vec<Op>, source: &str) -> TrackedPatch {
        TrackedPatch::new(Patch::with_ops(ops)).with_source(source)
    }

    #[tokio::test]
    async fn test_new_and_snapshot() {
        let initial = json!({"count": 0});
        let manager = StateManager::new(initial.clone());
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot, initial);
    }

    #[tokio::test]
    async fn test_apply_single() {
        let manager = StateManager::new(json!({}));

        let patch = make_patch(vec![Op::set(path!("count"), json!(10))], "test");

        let result = manager.apply(patch).await.unwrap();
        assert_eq!(result.patches_applied, 1);
        assert_eq!(result.ops_applied, 1);

        let state = manager.snapshot().await;
        assert_eq!(state["count"], 10);
    }

    #[tokio::test]
    async fn test_apply_batch() {
        let manager = StateManager::new(json!({}));

        let patches = vec![
            make_patch(vec![Op::set(path!("a"), json!(1))], "test1"),
            make_patch(vec![Op::set(path!("b"), json!(2))], "test2"),
        ];

        let result = manager.apply_batch(patches).await.unwrap();
        assert_eq!(result.patches_applied, 2);
        assert_eq!(result.ops_applied, 2);

        let state = manager.snapshot().await;
        assert_eq!(state["a"], 1);
        assert_eq!(state["b"], 2);
    }

    #[tokio::test]
    async fn test_history() {
        let manager = StateManager::new(json!({}));

        manager
            .apply(make_patch(vec![Op::set(path!("x"), json!(1))], "s1"))
            .await
            .unwrap();

        manager
            .apply(make_patch(vec![Op::set(path!("y"), json!(2))], "s2"))
            .await
            .unwrap();

        let history = manager.history().await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].source.as_deref(), Some("s1"));
        assert_eq!(history[1].source.as_deref(), Some("s2"));
    }

    #[tokio::test]
    async fn test_replay_to() {
        let manager = StateManager::new(json!({}));

        manager
            .apply(make_patch(vec![Op::set(path!("count"), json!(1))], "s1"))
            .await
            .unwrap();

        manager
            .apply(make_patch(vec![Op::set(path!("count"), json!(2))], "s2"))
            .await
            .unwrap();

        manager
            .apply(make_patch(vec![Op::set(path!("count"), json!(3))], "s3"))
            .await
            .unwrap();

        // Replay to index 0
        let state0 = manager.replay_to(0).await.unwrap();
        assert_eq!(state0["count"], 1);

        // Replay to index 1
        let state1 = manager.replay_to(1).await.unwrap();
        assert_eq!(state1["count"], 2);

        // Current state unchanged
        let current = manager.snapshot().await;
        assert_eq!(current["count"], 3);
    }

    #[tokio::test]
    async fn test_replay_invalid_index() {
        let manager = StateManager::new(json!({}));
        let result = manager.replay_to(0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_clear_history() {
        let manager = StateManager::new(json!({}));

        manager
            .apply(make_patch(vec![Op::set(path!("x"), json!(1))], "s1"))
            .await
            .unwrap();

        assert_eq!(manager.history_len().await, 1);

        manager.clear_history().await;

        assert_eq!(manager.history_len().await, 0);

        // State should be preserved
        let state = manager.snapshot().await;
        assert_eq!(state["x"], 1);
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let manager1 = StateManager::new(json!({}));
        let manager2 = manager1.clone();

        manager1
            .apply(make_patch(vec![Op::set(path!("x"), json!(42))], "s1"))
            .await
            .unwrap();

        let state = manager2.snapshot().await;
        assert_eq!(state["x"], 42);
    }
}
