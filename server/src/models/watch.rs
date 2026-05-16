use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::RecordId;

/// One persistent watch room per `Watch` channel. Keyed by channel id, so the
/// record id is `watch_room:<channel_id>`. Holds the durable slice of state
/// that needs to survive server restarts; transient playback (current
/// position while playing) is computed by clients from `playback_updated_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchRoom {
    pub id: Option<RecordId>,
    pub channel: RecordId,
    /// Current room leader. `None` while the room has no record yet or after
    /// the last member leaves and the actor evicts; on next subscribe the
    /// first joiner is promoted.
    pub leader: Option<RecordId>,
    /// The queue item currently playing, or `None` if the queue is empty.
    pub current_item: Option<RecordId>,
    pub playback_paused: bool,
    pub playback_position_ms: i64,
    pub playback_updated_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: Option<RecordId>,
    pub room: RecordId,
    /// YouTube video id (the 11-char `v=` parameter). Only YouTube is
    /// supported for Phase 4 — the player layer assumes this.
    pub video_id: String,
    pub title: String,
    pub duration_ms: i64,
    pub thumbnail_url: Option<String>,
    pub added_by: RecordId,
    /// Cached sum of votes, refreshed on every vote write.
    pub score: i32,
    /// Stable sort hint; ties broken by `created_at` ascending.
    pub position: i32,
    pub created_at: Option<DateTime<Utc>>,
}

