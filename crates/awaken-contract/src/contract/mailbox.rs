//! Mailbox job types and persistent queue trait for lease-based distributed claim.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::message::Message;
use super::storage::StorageError;

// ── data types ───────────────────────────────────────────────────────

/// A run request persisted in the mailbox queue.
///
/// Every run — streaming, background, A2A, internal notification —
/// enters the system as a MailboxJob keyed by thread_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxJob {
    // ── identity ──
    /// UUID v7, globally unique.
    pub job_id: String,
    /// = thread_id, routing anchor.
    pub mailbox_id: String,

    // ── request payload ──
    /// Target agent.
    pub agent_id: String,
    /// User/system messages.
    pub messages: Vec<Message>,
    /// Origin of the job.
    pub origin: MailboxJobOrigin,
    /// Audit / reply routing.
    pub sender_id: Option<String>,
    /// Parent-child run linkage.
    pub parent_run_id: Option<String>,
    /// InferenceOverride, serialized.
    pub overrides: Option<Value>,

    // ── queue semantics ──
    /// 0 = highest, 255 = lowest, default 128.
    pub priority: u8,
    /// Idempotent delivery key.
    pub dedupe_key: Option<String>,
    /// Mailbox generation (set by store on enqueue).
    pub generation: u64,

    // ── lifecycle ──
    /// Current status of this job.
    pub status: MailboxJobStatus,
    /// Unix millis; future = delayed delivery.
    pub available_at: u64,
    /// Number of claim attempts so far.
    pub attempt_count: u32,
    /// Default 5; exceeded -> DeadLetter.
    pub max_attempts: u32,
    /// Last error message (set on nack / dead_letter).
    pub last_error: Option<String>,

    // ── lease ──
    /// UUID, set on claim.
    pub claim_token: Option<String>,
    /// Consumer ID (process identifier).
    pub claimed_by: Option<String>,
    /// Unix millis, extended by heartbeat.
    pub lease_until: Option<u64>,

    // ── timestamps ──
    /// Unix millis when the job was created.
    pub created_at: u64,
    /// Unix millis of the last update.
    pub updated_at: u64,
}

/// Six-state lifecycle for mailbox jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailboxJobStatus {
    Queued,
    Claimed,
    Accepted,
    Cancelled,
    Superseded,
    DeadLetter,
}

impl MailboxJobStatus {
    /// Returns `true` for terminal states that cannot transition further.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::Cancelled | Self::Superseded | Self::DeadLetter
        )
    }
}

/// Origin of a mailbox job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailboxJobOrigin {
    /// HTTP API, SDK.
    User,
    /// Agent-to-Agent protocol.
    A2A,
    /// Child run completion notification, handoff.
    Internal,
}

/// Result of an interrupt operation.
#[derive(Debug, Clone)]
pub struct MailboxInterrupt {
    /// New generation after bump.
    pub new_generation: u64,
    /// The job that was Claimed (running) at interrupt time, if any.
    /// Caller should cancel the corresponding runtime run.
    pub active_job: Option<MailboxJob>,
    /// Number of Queued jobs superseded.
    pub superseded_count: usize,
}

// ── trait ─────────────────────────────────────────────────────────────

/// Persistent mailbox queue with lease-based distributed claim.
///
/// Implementations must guarantee:
/// - enqueue is durable before returning
/// - claim is atomic (exactly one consumer wins)
/// - interrupt atomically bumps generation + supersedes stale jobs
/// - ack/nack/dead_letter validate claim_token (reject stale claims)
#[async_trait]
pub trait MailboxStore: Send + Sync {
    // ── write path ──

    /// Persist a job. Sets generation from current MailboxState
    /// (auto-creates MailboxState if first job for this mailbox_id).
    /// Rejects if dedupe_key matches an existing non-terminal job.
    async fn enqueue(&self, job: &MailboxJob) -> Result<(), StorageError>;

    /// Atomically claim up to `limit` Queued jobs for a mailbox
    /// where `available_at <= now`. Sets status=Claimed, claim_token,
    /// claimed_by, lease_until = now + lease_ms.
    /// Returns claimed jobs ordered by (priority ASC, created_at ASC).
    async fn claim(
        &self,
        mailbox_id: &str,
        consumer_id: &str,
        lease_ms: u64,
        now: u64,
        limit: usize,
    ) -> Result<Vec<MailboxJob>, StorageError>;

    /// Claim a specific job by job_id. Same semantics as claim()
    /// but targets a single known job (used for inline streaming).
    async fn claim_job(
        &self,
        job_id: &str,
        consumer_id: &str,
        lease_ms: u64,
        now: u64,
    ) -> Result<Option<MailboxJob>, StorageError>;

    /// Mark job as successfully processed. Validates claim_token.
    async fn ack(&self, job_id: &str, claim_token: &str, now: u64) -> Result<(), StorageError>;

    /// Return job to queue for retry. Sets available_at = retry_at,
    /// increments attempt_count, records error.
    /// If attempt_count >= max_attempts, transitions to DeadLetter instead.
    async fn nack(
        &self,
        job_id: &str,
        claim_token: &str,
        retry_at: u64,
        error: &str,
        now: u64,
    ) -> Result<(), StorageError>;

    /// Permanently fail a job. Terminal state.
    async fn dead_letter(
        &self,
        job_id: &str,
        claim_token: &str,
        error: &str,
        now: u64,
    ) -> Result<(), StorageError>;

    /// Cancel a specific job. Works on Queued jobs only.
    /// For Claimed jobs, caller must also cancel the runtime run.
    async fn cancel(&self, job_id: &str, now: u64) -> Result<Option<MailboxJob>, StorageError>;

    /// Extend an active lease. Returns false if job not Claimed
    /// or claim_token mismatch (lease already expired and reclaimed).
    async fn extend_lease(
        &self,
        job_id: &str,
        claim_token: &str,
        extension_ms: u64,
        now: u64,
    ) -> Result<bool, StorageError>;

    // ── interrupt ──

    /// Atomically: bump generation, supersede all Queued jobs
    /// with generation < new_generation, return the Claimed job
    /// (if any) so caller can cancel its runtime run.
    async fn interrupt(&self, mailbox_id: &str, now: u64)
    -> Result<MailboxInterrupt, StorageError>;

    // ── read path ──

    /// Load a single job by ID.
    async fn load_job(&self, job_id: &str) -> Result<Option<MailboxJob>, StorageError>;

    /// List jobs for a mailbox, filtered by status.
    async fn list_jobs(
        &self,
        mailbox_id: &str,
        status_filter: Option<&[MailboxJobStatus]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxJob>, StorageError>;

    // ── maintenance ──

    /// Reclaim jobs whose lease_until < now (orphaned by crashed consumers).
    /// Resets to Queued with incremented attempt_count.
    /// Returns reclaimed jobs for immediate dispatch.
    async fn reclaim_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<MailboxJob>, StorageError>;

    /// Purge terminal jobs (Accepted, Cancelled, Superseded, DeadLetter)
    /// older than `older_than` timestamp. Returns count purged.
    async fn purge_terminal(&self, older_than: u64) -> Result<usize, StorageError>;

    /// List distinct mailbox_ids that have at least one Queued job.
    /// Used by recover() at startup.
    async fn queued_mailbox_ids(&self) -> Result<Vec<String>, StorageError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_terminal() {
        assert!(!MailboxJobStatus::Queued.is_terminal());
        assert!(!MailboxJobStatus::Claimed.is_terminal());
        assert!(MailboxJobStatus::Accepted.is_terminal());
        assert!(MailboxJobStatus::Cancelled.is_terminal());
        assert!(MailboxJobStatus::Superseded.is_terminal());
        assert!(MailboxJobStatus::DeadLetter.is_terminal());
    }

    #[test]
    fn job_serde_roundtrip() {
        let job = MailboxJob {
            job_id: "j-1".to_string(),
            mailbox_id: "m-1".to_string(),
            agent_id: "agent-1".to_string(),
            messages: vec![],
            origin: MailboxJobOrigin::User,
            sender_id: None,
            parent_run_id: None,
            overrides: None,
            priority: 128,
            dedupe_key: None,
            generation: 0,
            status: MailboxJobStatus::Queued,
            available_at: 1000,
            attempt_count: 0,
            max_attempts: 5,
            last_error: None,
            claim_token: None,
            claimed_by: None,
            lease_until: None,
            created_at: 1000,
            updated_at: 1000,
        };
        let json = serde_json::to_string(&job).unwrap();
        let parsed: MailboxJob = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.job_id, "j-1");
        assert_eq!(parsed.mailbox_id, "m-1");
        assert_eq!(parsed.status, MailboxJobStatus::Queued);
        assert_eq!(parsed.origin, MailboxJobOrigin::User);
    }

    #[test]
    fn status_serde_roundtrip() {
        for status in [
            MailboxJobStatus::Queued,
            MailboxJobStatus::Claimed,
            MailboxJobStatus::Accepted,
            MailboxJobStatus::Cancelled,
            MailboxJobStatus::Superseded,
            MailboxJobStatus::DeadLetter,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: MailboxJobStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn origin_serde_roundtrip() {
        for origin in [
            MailboxJobOrigin::User,
            MailboxJobOrigin::A2A,
            MailboxJobOrigin::Internal,
        ] {
            let json = serde_json::to_string(&origin).unwrap();
            let parsed: MailboxJobOrigin = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, origin);
        }
    }
}
