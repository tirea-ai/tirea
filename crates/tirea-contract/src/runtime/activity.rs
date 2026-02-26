//! Activity management trait for external state updates.

use serde_json::Value;
use tirea_state::Op;

/// Manager for activity state updates.
///
/// Implementations keep per-stream activity state and may emit external events.
pub trait ActivityManager: Send + Sync {
    /// Get the current activity snapshot for a stream.
    fn snapshot(&self, stream_id: &str) -> Value;

    /// Handle an activity operation for a stream.
    fn on_activity_op(&self, stream_id: &str, activity_type: &str, op: &Op);
}

#[cfg(test)]
mod tests {
    use crate::runtime::run::InferenceErrorState;
    use crate::testing::TestFixture;
    use serde_json::json;

    // Write-through same-ref and cross-ref covered by tool_call::context::tests.

    #[test]
    fn test_rebuild_state_reflects_write_through() {
        let doc = json!({"__inference_error": {"error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        // Write via state_of
        let ctrl = ctx.state_of::<InferenceErrorState>();
        ctrl.set_error(Some(crate::runtime::run::InferenceError {
            error_type: "test_error".into(),
            message: "test".into(),
        }))
        .expect("failed to set inference_error");

        // updated_state should return the run_doc snapshot which includes the write
        let rebuilt = fix.updated_state();
        assert_eq!(
            rebuilt["__inference_error"]["error"]["type"], "test_error",
            "updated_state must reflect write-through updates"
        );
    }
}
