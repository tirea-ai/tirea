//! Activity state manager and event emission.

use crate::runtime::streaming::AgentEvent;
use carve_state::{apply_patch, ActivityManager, Op, Patch, Value};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone)]
struct ActivityEntry {
    activity_type: String,
    state: Value,
}

/// Activity manager that keeps per-stream activity state and emits events on updates.
#[derive(Debug)]
pub struct ActivityHub {
    sender: UnboundedSender<AgentEvent>,
    entries: Mutex<HashMap<String, ActivityEntry>>,
}

impl ActivityHub {
    /// Create a new activity hub.
    pub fn new(sender: UnboundedSender<AgentEvent>) -> Self {
        Self {
            sender,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn entry_for(&self, _stream_id: &str, activity_type: &str) -> ActivityEntry {
        ActivityEntry {
            activity_type: activity_type.to_string(),
            state: json!({}),
        }
    }
}

impl ActivityManager for ActivityHub {
    fn snapshot(&self, stream_id: &str) -> Value {
        self.entries
            .lock()
            .unwrap()
            .get(stream_id)
            .map(|entry| entry.state.clone())
            .unwrap_or_else(|| json!({}))
    }

    fn on_activity_op(&self, stream_id: &str, activity_type: &str, op: &Op) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .entry(stream_id.to_string())
            .or_insert_with(|| self.entry_for(stream_id, activity_type));

        if entry.activity_type.is_empty() {
            entry.activity_type = activity_type.to_string();
        }

        let patch = Patch::with_ops(vec![op.clone()]);
        if let Ok(updated) = apply_patch(&entry.state, &patch) {
            entry.state = updated;
        }

        let _ = self.sender.send(AgentEvent::ActivitySnapshot {
            message_id: stream_id.to_string(),
            activity_type: entry.activity_type.clone(),
            content: entry.state.clone(),
            replace: Some(true),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::streaming::AgentEvent;
    use carve_state::{Op, Path};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[test]
    fn test_activity_hub_emits_snapshot_and_updates_state() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op = Op::set(Path::root().key("progress"), json!(0.2));
        hub.on_activity_op("stream_1", "progress", &op);

        let event = rx.try_recv().expect("activity event");
        match event {
            AgentEvent::ActivitySnapshot {
                message_id,
                activity_type,
                content,
                replace,
            } => {
                assert_eq!(message_id, "stream_1");
                assert_eq!(activity_type, "progress");
                assert_eq!(content["progress"], 0.2);
                assert_eq!(replace, Some(true));
            }
            _ => panic!("Expected ActivitySnapshot"),
        }

        let snapshot = hub.snapshot("stream_1");
        assert_eq!(snapshot["progress"], 0.2);
    }

    #[test]
    fn test_activity_hub_accumulates_updates() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op1 = Op::set(Path::root().key("progress"), json!(0.5));
        hub.on_activity_op("stream_2", "progress", &op1);
        let _ = rx.try_recv().expect("first event");

        let op2 = Op::set(Path::root().key("status"), json!("running"));
        hub.on_activity_op("stream_2", "progress", &op2);

        let event = rx.try_recv().expect("second event");
        match event {
            AgentEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content["progress"], 0.5);
                assert_eq!(content["status"], "running");
            }
            _ => panic!("Expected ActivitySnapshot"),
        }
    }

    #[test]
    fn test_activity_hub_preserves_activity_type() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op1 = Op::set(Path::root().key("progress"), json!(0.3));
        hub.on_activity_op("stream_3", "progress", &op1);
        let _ = rx.try_recv().expect("first event");

        let op2 = Op::set(Path::root().key("status"), json!("running"));
        hub.on_activity_op("stream_3", "other", &op2);

        let event = rx.try_recv().expect("second event");
        match event {
            AgentEvent::ActivitySnapshot { activity_type, .. } => {
                assert_eq!(activity_type, "progress");
            }
            _ => panic!("Expected ActivitySnapshot"),
        }
    }

    #[test]
    fn test_activity_hub_multiple_streams_isolated() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op1 = Op::set(Path::root().key("progress"), json!(0.1));
        hub.on_activity_op("stream_a", "progress", &op1);
        let _ = rx.try_recv().expect("event stream_a");

        let op2 = Op::set(Path::root().key("progress"), json!(0.9));
        hub.on_activity_op("stream_b", "progress", &op2);
        let _ = rx.try_recv().expect("event stream_b");

        let snapshot_a = hub.snapshot("stream_a");
        let snapshot_b = hub.snapshot("stream_b");

        assert_eq!(snapshot_a["progress"], 0.1);
        assert_eq!(snapshot_b["progress"], 0.9);
    }

    #[test]
    fn test_activity_hub_allows_scalar_state() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op = Op::set(Path::root(), json!("ok"));
        hub.on_activity_op("stream_scalar", "status", &op);

        let event = rx.try_recv().expect("event scalar");
        match event {
            AgentEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content, json!("ok"));
            }
            _ => panic!("Expected ActivitySnapshot"),
        }

        let snapshot = hub.snapshot("stream_scalar");
        assert_eq!(snapshot, json!("ok"));
    }

    #[test]
    fn test_activity_hub_allows_array_root_state() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op = Op::set(Path::root(), json!([1, 2, 3]));
        hub.on_activity_op("stream_array", "list", &op);

        let event = rx.try_recv().expect("event array");
        match event {
            AgentEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content, json!([1, 2, 3]));
            }
            _ => panic!("Expected ActivitySnapshot"),
        }

        let snapshot = hub.snapshot("stream_array");
        assert_eq!(snapshot, json!([1, 2, 3]));
    }

    #[test]
    fn test_activity_hub_invalid_op_keeps_state() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op = Op::increment(Path::root().key("progress"), 1);
        hub.on_activity_op("stream_invalid", "progress", &op);

        let event = rx.try_recv().expect("activity event");
        match event {
            AgentEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content, json!({}));
            }
            _ => panic!("Expected ActivitySnapshot"),
        }

        let snapshot = hub.snapshot("stream_invalid");
        assert_eq!(snapshot, json!({}));
    }

    #[test]
    fn test_activity_hub_emits_events_in_order() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op1 = Op::set(Path::root().key("progress"), json!(0.1));
        let op2 = Op::set(Path::root().key("progress"), json!(0.2));
        hub.on_activity_op("stream_order", "progress", &op1);
        hub.on_activity_op("stream_order", "progress", &op2);

        let first = rx.try_recv().expect("first event");
        let second = rx.try_recv().expect("second event");

        match first {
            AgentEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content["progress"], 0.1);
            }
            _ => panic!("Expected ActivitySnapshot"),
        }

        match second {
            AgentEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content["progress"], 0.2);
            }
            _ => panic!("Expected ActivitySnapshot"),
        }
    }

    #[test]
    fn test_activity_hub_emits_on_noop_update() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = ActivityHub::new(tx);

        let op = Op::set(Path::root().key("progress"), json!(0.5));
        hub.on_activity_op("stream_noop", "progress", &op);
        hub.on_activity_op("stream_noop", "progress", &op);

        let _ = rx.try_recv().expect("first event");
        let second = rx.try_recv().expect("second event");

        match second {
            AgentEvent::ActivitySnapshot { content, .. } => {
                assert_eq!(content["progress"], 0.5);
            }
            _ => panic!("Expected ActivitySnapshot"),
        }
    }

    #[tokio::test]
    async fn test_activity_hub_concurrent_updates_merge() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hub = Arc::new(ActivityHub::new(tx));

        let mut handles = Vec::new();
        for i in 0..5 {
            let hub = hub.clone();
            handles.push(tokio::spawn(async move {
                let op = Op::set(Path::root().key(format!("k{}", i)), json!(i));
                hub.on_activity_op("stream_concurrent", "progress", &op);
            }));
        }

        for handle in handles {
            handle.await.expect("task");
        }

        while rx.try_recv().is_ok() {}

        let snapshot = hub.snapshot("stream_concurrent");
        for i in 0..5 {
            assert_eq!(snapshot[format!("k{}", i)], i);
        }
    }
}
