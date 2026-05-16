use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::error::AppError;
use crate::models::watch::{QueueItem, WatchRoom};

#[derive(Debug, Serialize)]
struct CreateWatchRoomDb {
    channel: surrealdb::RecordId,
    leader: Option<surrealdb::RecordId>,
    current_item: Option<surrealdb::RecordId>,
    playback_paused: bool,
    playback_position_ms: i64,
    playback_updated_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct CreateQueueItemDb {
    room: surrealdb::RecordId,
    video_id: String,
    title: String,
    duration_ms: i64,
    thumbnail_url: Option<String>,
    added_by: surrealdb::RecordId,
    score: i32,
    position: i32,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct CountResult {
    count: u64,
}

/// Snapshot of authoritative playback state, passed when persisting from the
/// in-memory actor. Kept separate from `PlaybackState` (the WS-facing struct)
/// so the persistence path doesn't carry the `video_id` redundantly — the
/// current item is tracked via the `current_item` edge already.
pub struct PlaybackPersist {
    pub current_item_id: Option<String>,
    pub position_ms: i64,
    pub paused: bool,
}

/// Persistence for watch rooms and their queues. A watch channel has exactly
/// one `watch_room` record keyed by the channel id (`watch_room:<channel_id>`).
#[cfg_attr(test, automock)]
#[async_trait]
pub trait WatchRepo: Send + Sync {
    /// Idempotent: create the room if it doesn't already exist. Returns the
    /// (possibly pre-existing) row.
    async fn ensure_room(&self, channel_id: &str) -> Result<WatchRoom, AppError>;

    async fn find_room(&self, channel_id: &str) -> Result<Option<WatchRoom>, AppError>;

    /// Persist playback + leader after debounced actor flushes.
    async fn save_playback(
        &self,
        channel_id: &str,
        leader_id: Option<String>,
        playback: PlaybackPersist,
    ) -> Result<(), AppError>;

    /// Append a queue item at the next position slot. Score starts at 0.
    async fn add_queue_item(
        &self,
        channel_id: &str,
        added_by: &str,
        video_id: String,
        title: String,
        duration_ms: i64,
        thumbnail_url: Option<String>,
    ) -> Result<QueueItem, AppError>;

    async fn remove_queue_item(&self, item_id: &str) -> Result<(), AppError>;

    /// Full queue for a room, ordered by score desc, then created_at asc.
    async fn list_queue(&self, channel_id: &str) -> Result<Vec<QueueItem>, AppError>;

    async fn find_queue_item(&self, item_id: &str) -> Result<Option<QueueItem>, AppError>;

    /// Set this user's vote on an item. `value` must be -1, 0, or 1; 0 removes
    /// the vote. Returns the queue item's new total score after the change.
    async fn set_vote(
        &self,
        user_id: &str,
        item_id: &str,
        value: i32,
    ) -> Result<i32, AppError>;

    /// Upsert a `watched` edge from the user to the YouTube video. Dedupes by
    /// `(user, video_id)` — bumps `watch_count` + `last_watched` if exists.
    async fn record_watched(
        &self,
        user_id: &str,
        video_id: &str,
        completion_pct: f64,
    ) -> Result<(), AppError>;

    /// Returns true if this user already has a `watched` edge to the video,
    /// regardless of completion. Used as a recommendation filter.
    async fn has_watched(&self, user_id: &str, video_id: &str) -> Result<bool, AppError>;
}

pub struct SurrealWatchRepo {
    db: Surreal<Client>,
}

impl SurrealWatchRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl WatchRepo for SurrealWatchRepo {
    async fn ensure_room(&self, channel_id: &str) -> Result<WatchRoom, AppError> {
        if let Some(existing) = self.find_room(channel_id).await? {
            return Ok(existing);
        }
        let now = chrono::Utc::now();
        let created: Result<Option<WatchRoom>, _> = self
            .db
            .create(("watch_room", channel_id))
            .content(CreateWatchRoomDb {
                channel: surrealdb::RecordId::from(("channel", channel_id)),
                leader: None,
                current_item: None,
                playback_paused: true,
                playback_position_ms: 0,
                playback_updated_at: now,
                created_at: now,
                updated_at: now,
            })
            .await;
        match created {
            Ok(Some(room)) => Ok(room),
            Ok(None) => Err(AppError::Internal(
                "watch_room create returned no row".into(),
            )),
            // Lost a concurrent create race — refetch.
            Err(_) => self
                .find_room(channel_id)
                .await?
                .ok_or_else(|| AppError::Internal("watch_room missing after create race".into())),
        }
    }

    async fn find_room(&self, channel_id: &str) -> Result<Option<WatchRoom>, AppError> {
        let room: Option<WatchRoom> = self.db.select(("watch_room", channel_id)).await?;
        Ok(room)
    }

    async fn save_playback(
        &self,
        channel_id: &str,
        leader_id: Option<String>,
        playback: PlaybackPersist,
    ) -> Result<(), AppError> {
        let leader = leader_id
            .as_deref()
            .map(|id| surrealdb::RecordId::from(("user", id)));
        let current_item = playback
            .current_item_id
            .as_deref()
            .map(|id| surrealdb::RecordId::from(("watch_queue_item", id)));
        self.db
            .query(
                "UPDATE $id SET leader = $leader, current_item = $current, \
                 playback_paused = $paused, playback_position_ms = $pos, \
                 playback_updated_at = time::now(), updated_at = time::now()",
            )
            .bind(("id", surrealdb::RecordId::from(("watch_room", channel_id))))
            .bind(("leader", leader))
            .bind(("current", current_item))
            .bind(("paused", playback.paused))
            .bind(("pos", playback.position_ms))
            .await?;
        Ok(())
    }

    async fn add_queue_item(
        &self,
        channel_id: &str,
        added_by: &str,
        video_id: String,
        title: String,
        duration_ms: i64,
        thumbnail_url: Option<String>,
    ) -> Result<QueueItem, AppError> {
        // Compute next position as max(position) + 1 in a single round-trip.
        // Falls back to 0 when the queue is empty.
        let mut q = self
            .db
            .query("SELECT math::max(position) AS max_pos FROM watch_queue_item WHERE room = $room")
            .bind((
                "room",
                surrealdb::RecordId::from(("watch_room", channel_id)),
            ))
            .await?;
        #[derive(Deserialize)]
        struct MaxRow {
            max_pos: Option<i32>,
        }
        let rows: Vec<MaxRow> = q.take(0)?;
        let next_pos = rows.first().and_then(|r| r.max_pos).map(|p| p + 1).unwrap_or(0);

        let now = chrono::Utc::now();
        let created: Option<QueueItem> = self
            .db
            .create("watch_queue_item")
            .content(CreateQueueItemDb {
                room: surrealdb::RecordId::from(("watch_room", channel_id)),
                video_id,
                title,
                duration_ms,
                thumbnail_url,
                added_by: surrealdb::RecordId::from(("user", added_by)),
                score: 0,
                position: next_pos,
                created_at: now,
            })
            .await?;
        created.ok_or_else(|| AppError::Internal("Failed to insert queue item".into()))
    }

    async fn remove_queue_item(&self, item_id: &str) -> Result<(), AppError> {
        self.db
            .query("DELETE $id")
            .bind((
                "id",
                surrealdb::RecordId::from(("watch_queue_item", item_id)),
            ))
            .await?;
        Ok(())
    }

    async fn list_queue(&self, channel_id: &str) -> Result<Vec<QueueItem>, AppError> {
        let mut q = self
            .db
            .query(
                "SELECT * FROM watch_queue_item WHERE room = $room \
                 ORDER BY score DESC, created_at ASC",
            )
            .bind((
                "room",
                surrealdb::RecordId::from(("watch_room", channel_id)),
            ))
            .await?;
        let items: Vec<QueueItem> = q.take(0)?;
        Ok(items)
    }

    async fn find_queue_item(&self, item_id: &str) -> Result<Option<QueueItem>, AppError> {
        let item: Option<QueueItem> = self
            .db
            .select(("watch_queue_item", item_id))
            .await?;
        Ok(item)
    }

    async fn set_vote(
        &self,
        user_id: &str,
        item_id: &str,
        value: i32,
    ) -> Result<i32, AppError> {
        if !(-1..=1).contains(&value) {
            return Err(AppError::BadRequest("vote value must be -1, 0, or 1".into()));
        }
        let user = surrealdb::RecordId::from(("user", user_id));
        let item = surrealdb::RecordId::from(("watch_queue_item", item_id));

        // Run DELETE + (optional) RELATE + score recompute as one transaction
        // so concurrent voters can't (a) wipe each other's edges mid-flight or
        // (b) read a partial sum into the cached `score`. Surreal's BEGIN
        // /COMMIT serializes the inner statements against the affected rows.
        let mut q = if value != 0 {
            self.db
                .query("BEGIN;")
                .query("DELETE votes WHERE in = $user AND out = $item;")
                .query(
                    "RELATE $user -> votes -> $item SET value = $value, created_at = time::now();",
                )
                .query(
                    "LET $total = (SELECT VALUE math::sum(value) FROM votes WHERE out = $item)[0] ?? 0; \
                     UPDATE $item SET score = $total RETURN score;",
                )
                .query("COMMIT;")
                .bind(("user", user))
                .bind(("item", item))
                .bind(("value", value))
                .await?
        } else {
            self.db
                .query("BEGIN;")
                .query("DELETE votes WHERE in = $user AND out = $item;")
                .query(
                    "LET $total = (SELECT VALUE math::sum(value) FROM votes WHERE out = $item)[0] ?? 0; \
                     UPDATE $item SET score = $total RETURN score;",
                )
                .query("COMMIT;")
                .bind(("user", user))
                .bind(("item", item))
                .await?
        };

        #[derive(Deserialize)]
        struct ScoreRow {
            score: i32,
        }
        // The score row comes from the last data-returning statement before
        // COMMIT. We probe a few indices because Surreal's response numbering
        // depends on whether transactional statements report their own slots.
        for idx in [3usize, 2, 1, 0] {
            if let Ok(rows) = q.take::<Vec<ScoreRow>>(idx) {
                if let Some(row) = rows.into_iter().next() {
                    return Ok(row.score);
                }
            }
        }
        Ok(0)
    }

    async fn record_watched(
        &self,
        user_id: &str,
        video_id: &str,
        completion_pct: f64,
    ) -> Result<(), AppError> {
        // Dedupe by (user, video_id): bump watch_count + last_watched on the
        // existing edge if present, otherwise RELATE a fresh one. Run inside
        // a transaction so two concurrent `record_watched` calls for the same
        // (user, video) can't both take the insert path and produce duplicate
        // edges. The `(completion_pct ?? 0)` guard avoids relying on Surreal
        // identifier-shadowing semantics for the LHS of `math::max`.
        let user = surrealdb::RecordId::from(("user", user_id));
        let media = surrealdb::RecordId::from(("media", video_id));
        self.db
            .query("BEGIN;")
            .query(
                "LET $existing = (SELECT id FROM watched \
                 WHERE in = $user AND video_id = $vid LIMIT 1)[0].id;",
            )
            .query(
                "IF $existing != NONE THEN \
                    UPDATE $existing SET watch_count = watch_count + 1, \
                        last_watched = time::now(), \
                        completion_pct = math::max([(completion_pct ?? 0), $pct]) \
                 ELSE \
                    RELATE $user -> watched -> $media SET \
                        video_id = $vid, watch_count = 1, last_watched = time::now(), \
                        completion_pct = $pct, created_at = time::now() \
                 END;",
            )
            .query("COMMIT;")
            .bind(("user", user))
            .bind(("media", media))
            .bind(("vid", video_id.to_string()))
            .bind(("pct", completion_pct))
            .await?;
        Ok(())
    }

    async fn has_watched(&self, user_id: &str, video_id: &str) -> Result<bool, AppError> {
        let mut q = self
            .db
            .query(
                "SELECT count() AS count FROM watched \
                 WHERE in = $user AND video_id = $vid GROUP BY count",
            )
            .bind(("user", surrealdb::RecordId::from(("user", user_id))))
            .bind(("vid", video_id.to_string()))
            .await?;
        let counts: Vec<CountResult> = q.take(0)?;
        Ok(counts.first().map(|c| c.count > 0).unwrap_or(false))
    }
}
