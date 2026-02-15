//! NATS JetStream-buffered storage decorator.
//!
//! Wraps an inner [`ThreadStore`] (typically PostgreSQL) and routes delta
//! writes through NATS JetStream instead of hitting the database per-delta.
//!
//! # Run-end flush strategy
//!
//! During a run the [`AgentOs::run_stream`] checkpoint background task calls
//! `append()` for each delta.  This storage publishes those deltas to a
//! JetStream subject `thread.{thread_id}.deltas` so they are durably buffered
//! in NATS.  No database writes happen during the run.
//!
//! At run end `run_stream` calls `save()` with the final materialised thread,
//! which is forwarded to the inner storage as a single atomic write.  After the
//! write succeeds the corresponding NATS messages are acknowledged (purged).
//!
//! # Crash recovery
//!
//! On startup, call [`NatsBufferedStorage::recover`] to replay any unacked
//! deltas left over from interrupted runs.

use async_nats::jetstream;
use async_trait::async_trait;
use carve_agent::storage::{Committed, StorageError, ThreadDelta, ThreadHead, ThreadStore};
use carve_agent::Thread;
use std::sync::Arc;

/// NATS JetStream stream name for thread deltas.
const STREAM_NAME: &str = "THREAD_DELTAS";

/// Subject prefix.  Full subject: `thread.{thread_id}.deltas`.
const SUBJECT_PREFIX: &str = "thread";

fn delta_subject(thread_id: &str) -> String {
    format!("{SUBJECT_PREFIX}.{thread_id}.deltas")
}

/// A [`ThreadStore`] decorator that buffers deltas in NATS JetStream and
/// flushes the final thread to the inner storage at run end.
///
/// # Query consistency (CQRS)
///
/// [`load`](ThreadStore::load) always reads from the inner (durable) storage.
/// During an active run, queries return the **last-flushed snapshot** — they do
/// not include deltas that are buffered in NATS but not yet flushed.
///
/// Real-time data for in-progress runs is delivered through the SSE/NATS event
/// stream.  Callers that need up-to-date messages during a run should consume
/// the event stream rather than polling the query API.
pub struct NatsBufferedStorage {
    inner: Arc<dyn ThreadStore>,
    jetstream: jetstream::Context,
}

impl NatsBufferedStorage {
    /// Create a new buffered storage.
    ///
    /// `inner` is the durable backend (e.g. PostgreSQL) used for `create`,
    /// `load`, `delete`, and the final `save` at run end.
    ///
    /// `jetstream` is an already-connected JetStream context.
    pub async fn new(
        inner: Arc<dyn ThreadStore>,
        jetstream: jetstream::Context,
    ) -> Result<Self, async_nats::Error> {
        // Ensure the stream exists (idempotent).
        jetstream
            .get_or_create_stream(jetstream::stream::Config {
                name: STREAM_NAME.to_string(),
                subjects: vec![format!("{SUBJECT_PREFIX}.*.deltas")],
                retention: jetstream::stream::RetentionPolicy::WorkQueue,
                storage: jetstream::stream::StorageType::File,
                max_age: std::time::Duration::from_secs(24 * 3600), // 24h TTL
                ..Default::default()
            })
            .await?;

        Ok(Self { inner, jetstream })
    }

    /// Recover incomplete runs after a crash.
    ///
    /// Replays any unacked deltas from the JetStream stream, applies them to
    /// the corresponding threads loaded from the inner storage, and saves the
    /// result.  Acked messages are then purged.
    pub async fn recover(&self) -> Result<usize, NatsStorageError> {
        let stream = self
            .jetstream
            .get_stream(STREAM_NAME)
            .await
            .map_err(|e| NatsStorageError::JetStream(e.to_string()))?;

        let consumer = stream
            .create_consumer(jetstream::consumer::pull::Config {
                name: Some("recovery".to_string()),
                ack_policy: jetstream::consumer::AckPolicy::Explicit,
                deliver_policy: jetstream::consumer::DeliverPolicy::All,
                filter_subject: format!("{SUBJECT_PREFIX}.*.deltas"),
                ..Default::default()
            })
            .await
            .map_err(|e| NatsStorageError::JetStream(e.to_string()))?;

        let mut recovered = 0usize;
        // Collect pending deltas grouped by thread_id.
        let mut pending: std::collections::HashMap<String, Vec<(ThreadDelta, jetstream::Message)>> =
            std::collections::HashMap::new();

        // Fetch messages in batches.
        use futures::StreamExt;
        let mut messages = consumer
            .messages()
            .await
            .map_err(|e| NatsStorageError::JetStream(e.to_string()))?;

        // Use a short timeout to drain available messages.
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(500), messages.next()).await
            {
                Ok(Some(Ok(msg))) => {
                    let subject = msg.subject.to_string();
                    // Parse thread_id from subject: "thread.{thread_id}.deltas"
                    let parts: Vec<&str> = subject.split('.').collect();
                    if parts.len() != 3 {
                        // Unexpected subject format; ack and skip.
                        let _ = msg.ack().await;
                        continue;
                    }
                    let thread_id = parts[1].to_string();

                    match serde_json::from_slice::<ThreadDelta>(&msg.payload) {
                        Ok(delta) => {
                            pending.entry(thread_id).or_default().push((delta, msg));
                        }
                        Err(_) => {
                            // Malformed message; ack to avoid redelivery loop.
                            let _ = msg.ack().await;
                        }
                    }
                }
                Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
            }
        }

        // Replay each thread's deltas.
        for (thread_id, deltas_with_msgs) in pending {
            let mut thread = match self.inner.load(&thread_id).await {
                Ok(Some(head)) => head.thread,
                Ok(None) => Thread::new(thread_id.clone()),
                Err(e) => {
                    tracing::error!(
                        thread_id = %thread_id,
                        error = %e,
                        "recovery: failed to load thread, skipping"
                    );
                    continue;
                }
            };

            for (delta, _) in &deltas_with_msgs {
                apply_delta(&mut thread, delta);
            }

            if let Err(e) = self.inner.save(&thread).await {
                tracing::error!(
                    thread_id = %thread_id,
                    error = %e,
                    "recovery: failed to save recovered thread"
                );
                continue;
            }

            // Ack all messages for this thread.
            for (_, msg) in deltas_with_msgs {
                let _ = msg.ack().await;
                recovered += 1;
            }
        }

        // Delete the ephemeral recovery consumer.
        let _ = stream.delete_consumer("recovery").await;

        Ok(recovered)
    }
}

#[async_trait]
impl ThreadStore for NatsBufferedStorage {
    async fn create(&self, thread: &Thread) -> Result<Committed, StorageError> {
        self.inner.create(thread).await
    }

    /// Publish delta to NATS JetStream instead of writing to database.
    ///
    /// The delta is durably stored in JetStream and will be purged after the
    /// run-end `save()` succeeds.  If publishing fails the error is mapped to
    /// [`StorageError::Io`].
    async fn append(
        &self,
        thread_id: &str,
        delta: &ThreadDelta,
    ) -> Result<Committed, StorageError> {
        let payload =
            serde_json::to_vec(delta).map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.jetstream
            .publish(delta_subject(thread_id), payload.into())
            .await
            .map_err(|e| StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?
            .await
            .map_err(|e| StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Version is cosmetic here — the real version lives in the inner storage.
        Ok(Committed { version: 0 })
    }

    async fn load(&self, thread_id: &str) -> Result<Option<ThreadHead>, StorageError> {
        self.inner.load(thread_id).await
    }

    async fn delete(&self, thread_id: &str) -> Result<(), StorageError> {
        self.inner.delete(thread_id).await
    }

    /// Run-end flush: saves the final materialized thread to the inner storage
    /// and purges the corresponding NATS JetStream messages.
    async fn save(&self, thread: &Thread) -> Result<(), StorageError> {
        // Write to durable storage.
        self.inner.save(thread).await?;

        // Best-effort purge of buffered deltas for this thread.
        if let Ok(stream) = self.jetstream.get_stream(STREAM_NAME).await {
            let _ = stream.purge().filter(delta_subject(&thread.id)).await;
        }

        Ok(())
    }
}

/// Apply a delta to a thread in-place (same logic as storage::apply_delta but
/// accessible here without depending on the private function).
fn apply_delta(thread: &mut Thread, delta: &ThreadDelta) {
    thread.messages.extend(delta.messages.iter().cloned());
    thread.patches.extend(delta.patches.iter().cloned());
    if let Some(ref snapshot) = delta.snapshot {
        thread.state = snapshot.clone();
        thread.patches.clear();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NatsStorageError {
    #[error("jetstream error: {0}")]
    JetStream(String),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
}
