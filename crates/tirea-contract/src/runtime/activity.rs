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
    use crate::runtime::control::InferenceErrorState;
    use crate::testing::TestFixture;
    use serde_json::json;

    // ================================================================
    // Write-through-read tests
    // ================================================================

    #[test]
    fn test_write_through_read_same_ref() {
        let doc = json!({"__inference_error": {"error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        let ctrl = ctx.state_of::<InferenceErrorState>();
        // Initially null
        assert!(ctrl.error().unwrap().is_none());

        // Write
        ctrl.set_error(Some(crate::runtime::control::InferenceError {
            error_type: "rate_limit".into(),
            message: "too many requests".into(),
        }))
        .expect("failed to set inference_error");

        // Read back from the same ref — must see the written value
        let err = ctrl.error().unwrap();
        assert!(err.is_some(), "same-ref read must see the write");
        assert_eq!(err.unwrap().error_type, "rate_limit");
    }

    #[test]
    fn test_write_through_read_cross_ref() {
        let doc = json!({"__inference_error": {"error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        // Write via first state_of call
        let ctrl1 = ctx.state_of::<InferenceErrorState>();
        ctrl1
            .set_error(Some(crate::runtime::control::InferenceError {
                error_type: "timeout".into(),
                message: "timed out".into(),
            }))
            .expect("failed to set inference_error");

        // Read via second state_of call — must see the write
        let ctrl2 = ctx.state_of::<InferenceErrorState>();
        let err = ctrl2.error().unwrap();
        assert!(err.is_some(), "cross-ref read must see the write");
        assert_eq!(err.unwrap().error_type, "timeout");
    }

    #[test]
    fn test_rebuild_state_reflects_write_through() {
        let doc = json!({"__inference_error": {"error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        // Write via state_of
        let ctrl = ctx.state_of::<InferenceErrorState>();
        ctrl.set_error(Some(crate::runtime::control::InferenceError {
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
