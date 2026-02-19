//! Activity management trait for external state updates.

use carve_state::Op;
use serde_json::Value;

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
    use crate::runtime::control::LoopControlState;
    use crate::thread::Thread;
    use crate::testing::TestFixture;
    use carve_state::{path, Op};
    use serde_json::json;

    #[test]
    fn test_override_state_of_writes_to_overlay() {
        let doc = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let fix = TestFixture::new_with_state(doc);

        // Write via override — should go to run_overlay, not ops
        let ctx = fix.ctx_with("call-1", "test");
        let ctrl = ctx.override_state_of::<LoopControlState>();
        ctrl.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "rate_limit".into(),
            message: "too many requests".into(),
        }));

        assert!(
            fix.ops.lock().unwrap().is_empty(),
            "thread ops must remain empty after override write"
        );
        assert!(
            !fix.overlay.lock().unwrap().is_empty(),
            "overlay must contain the override op"
        );
    }

    #[test]
    fn test_state_of_reads_see_overlay_via_rebuild() {
        let initial = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let thread = Thread::with_initial_state("t-1", initial);

        // Write overlay ops directly
        thread.run_overlay.lock().unwrap().push(Op::set(
            path!("loop_control", "inference_error"),
            json!({"type": "timeout", "message": "timed out"}),
        ));

        // rebuild_state should include the overlay
        let rebuilt = thread.rebuild_state().unwrap();
        assert_eq!(
            rebuilt["loop_control"]["inference_error"]["type"], "timeout",
            "rebuild_state must include overlay values"
        );

        // rebuild_thread_state should NOT include it
        let thread_state = thread.rebuild_thread_state().unwrap();
        assert!(
            thread_state["loop_control"]["inference_error"].is_null(),
            "rebuild_thread_state must exclude overlay"
        );
    }

    // ================================================================
    // Write-through-read tests
    // ================================================================

    #[test]
    fn test_write_through_read_same_ref() {
        let doc = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        let ctrl = ctx.state_of::<LoopControlState>();
        // Initially null
        assert!(ctrl.inference_error().unwrap().is_none());

        // Write
        ctrl.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "rate_limit".into(),
            message: "too many requests".into(),
        }));

        // Read back from the same ref — must see the written value
        let err = ctrl.inference_error().unwrap();
        assert!(err.is_some(), "same-ref read must see the write");
        assert_eq!(err.unwrap().error_type, "rate_limit");
    }

    #[test]
    fn test_write_through_read_cross_ref() {
        let doc = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        // Write via first state_of call
        let ctrl1 = ctx.state_of::<LoopControlState>();
        ctrl1.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "timeout".into(),
            message: "timed out".into(),
        }));

        // Read via second state_of call — must see the write
        let ctrl2 = ctx.state_of::<LoopControlState>();
        let err = ctrl2.inference_error().unwrap();
        assert!(err.is_some(), "cross-ref read must see the write");
        assert_eq!(err.unwrap().error_type, "timeout");
    }

    #[test]
    fn test_override_write_visible_to_state_of_read() {
        let doc = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        // Write via override_state_of (overlay)
        let ctrl_override = ctx.override_state_of::<LoopControlState>();
        ctrl_override.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "overridden".into(),
            message: "from overlay".into(),
        }));

        // Read via state_of (thread ops path) — must see the override
        let ctrl = ctx.state_of::<LoopControlState>();
        let err = ctrl.inference_error().unwrap();
        assert!(
            err.is_some(),
            "state_of read must see override_state_of write"
        );
        assert_eq!(err.unwrap().error_type, "overridden");
    }

    #[test]
    fn test_state_of_write_visible_to_override_read() {
        let doc = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        // Write via state_of (thread ops)
        let ctrl = ctx.state_of::<LoopControlState>();
        ctrl.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "from_state_of".into(),
            message: "via thread ops".into(),
        }));

        // Read via override_state_of — must see the write
        let ctrl_override = ctx.override_state_of::<LoopControlState>();
        let err = ctrl_override.inference_error().unwrap();
        assert!(
            err.is_some(),
            "override_state_of read must see state_of write"
        );
        assert_eq!(err.unwrap().error_type, "from_state_of");
    }

    #[test]
    fn test_rebuild_state_reflects_write_through() {
        let doc = json!({"loop_control": {"pending_interaction": null, "inference_error": null}});
        let fix = TestFixture::new_with_state(doc);
        let ctx = fix.ctx_with("call-1", "test");

        // Write via state_of
        let ctrl = ctx.state_of::<LoopControlState>();
        ctrl.set_inference_error(Some(crate::runtime::control::InferenceError {
            error_type: "test_error".into(),
            message: "test".into(),
        }));

        // updated_state should return the run_doc snapshot which includes the write
        let rebuilt = fix.updated_state();
        assert_eq!(
            rebuilt["loop_control"]["inference_error"]["type"], "test_error",
            "updated_state must reflect write-through updates"
        );
    }
}
