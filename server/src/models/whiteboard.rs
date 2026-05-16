use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::RecordId;

/// Persistent whiteboard state. One row per whiteboard channel (1:1 with
/// the channel via the `channel` ref). Created lazily on first edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Whiteboard {
    pub id: Option<RecordId>,
    pub channel: RecordId,
    /// Serialized Yjs full state (base64 of v1 update bytes).
    pub state_b64: String,
    /// Latest Yjs state vector (base64). Reserved for diff sync.
    pub state_vector_b64: String,
    /// Total number of snapshots persisted — used to gate checkpoint writes
    /// (every Nth snapshot).
    pub snapshot_count: u64,
    pub last_snapshot_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

/// A frozen-in-time copy of the whiteboard state. Used for undo-to-checkpoint
/// restore. Kept on a rolling cap (oldest trimmed when over [`MAX_CHECKPOINTS`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhiteboardCheckpoint {
    pub id: Option<RecordId>,
    pub channel: RecordId,
    pub state_b64: String,
    pub label: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Max checkpoints retained per whiteboard (rolling window).
pub const MAX_CHECKPOINTS: u64 = 50;

/// Server-side cap on full whiteboard doc size after merge. Beyond this, the
/// most recent update is rejected with a `doc_too_large` error.
pub const WHITEBOARD_MAX_DOC_BYTES: usize = 4 * 1024 * 1024;

/// Per-update cap for whiteboards — wider than the post default to allow
/// streaming strokes with many points in one transaction.
pub const WHITEBOARD_MAX_UPDATE_BYTES: usize = 512 * 1024;
