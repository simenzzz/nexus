use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::error::AppError;
use crate::models::whiteboard::{Whiteboard, WhiteboardCheckpoint, MAX_CHECKPOINTS};

#[derive(Debug, Serialize)]
struct CreateWhiteboardDb {
    channel: surrealdb::RecordId,
    state_b64: String,
    state_vector_b64: String,
    snapshot_count: u64,
    last_snapshot_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct CreateCheckpointDb {
    channel: surrealdb::RecordId,
    state_b64: String,
    label: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    snapshot_count: u64,
}

/// Persistence for whiteboard channels. The 1:1 mapping between a whiteboard
/// channel and a `whiteboard` record uses the channel's ID as the whiteboard
/// record key (i.e. `whiteboard:<channel_id>`), so a single lookup serves
/// both find-by-channel and find-by-id.
#[cfg_attr(test, automock)]
#[async_trait]
pub trait WhiteboardRepo: Send + Sync {
    /// Fetch the persisted whiteboard for a channel, or `None` if it has
    /// never been edited.
    async fn find_by_channel(&self, channel_id: &str) -> Result<Option<Whiteboard>, AppError>;

    /// Upsert the full snapshot — creates the row on first save, then
    /// overwrites bytes + bumps `snapshot_count` on subsequent saves.
    /// Returns the updated `snapshot_count` so the caller can decide whether
    /// to also write a checkpoint.
    async fn upsert_snapshot(
        &self,
        channel_id: &str,
        state_b64: String,
        state_vector_b64: String,
    ) -> Result<u64, AppError>;

    /// Append a new checkpoint and trim the oldest entries over
    /// [`MAX_CHECKPOINTS`].
    async fn append_checkpoint(
        &self,
        channel_id: &str,
        state_b64: String,
        label: Option<String>,
    ) -> Result<WhiteboardCheckpoint, AppError>;

    /// List checkpoints for a whiteboard, newest first.
    async fn list_checkpoints(
        &self,
        channel_id: &str,
    ) -> Result<Vec<WhiteboardCheckpoint>, AppError>;

    /// Load a specific checkpoint by its record key.
    async fn find_checkpoint(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<WhiteboardCheckpoint>, AppError>;
}

pub struct SurrealWhiteboardRepo {
    db: Surreal<Client>,
}

impl SurrealWhiteboardRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl WhiteboardRepo for SurrealWhiteboardRepo {
    async fn find_by_channel(&self, channel_id: &str) -> Result<Option<Whiteboard>, AppError> {
        let wb: Option<Whiteboard> = self.db.select(("whiteboard", channel_id)).await?;
        Ok(wb)
    }

    async fn upsert_snapshot(
        &self,
        channel_id: &str,
        state_b64: String,
        state_vector_b64: String,
    ) -> Result<u64, AppError> {
        // Try an atomic UPDATE with `snapshot_count + 1` so concurrent
        // persists (e.g. sweeper flush racing with `flush_now`) can't lose
        // increments by both reading the same prior value. Returns the new
        // count via RETURN AFTER. If the row doesn't exist yet, UPDATE
        // returns no rows; fall through to CREATE.
        let id = surrealdb::RecordId::from(("whiteboard", channel_id));
        let mut q = self
            .db
            .query(
                "UPDATE $id SET state_b64 = $state, state_vector_b64 = $sv, \
                 snapshot_count = snapshot_count + 1, last_snapshot_at = time::now() \
                 RETURN snapshot_count",
            )
            .bind(("id", id))
            .bind(("state", state_b64.clone()))
            .bind(("sv", state_vector_b64.clone()))
            .await?;
        let updated: Vec<CountRow> = q.take(0)?;
        if let Some(row) = updated.into_iter().next() {
            return Ok(row.snapshot_count);
        }

        // First save: row doesn't exist. CREATE keyed by channel_id. If a
        // concurrent first-save raced us, the second CREATE errors with a
        // duplicate-key — retry the UPDATE path once to claim our increment.
        let now = chrono::Utc::now();
        let created: Result<Option<Whiteboard>, _> = self
            .db
            .create(("whiteboard", channel_id))
            .content(CreateWhiteboardDb {
                channel: surrealdb::RecordId::from(("channel", channel_id)),
                state_b64: state_b64.clone(),
                state_vector_b64: state_vector_b64.clone(),
                snapshot_count: 1,
                last_snapshot_at: now,
                created_at: now,
            })
            .await;
        match created {
            Ok(_) => Ok(1),
            Err(_) => {
                // Lost the create race — UPDATE now succeeds.
                let id = surrealdb::RecordId::from(("whiteboard", channel_id));
                let mut q = self
                    .db
                    .query(
                        "UPDATE $id SET state_b64 = $state, state_vector_b64 = $sv, \
                         snapshot_count = snapshot_count + 1, last_snapshot_at = time::now() \
                         RETURN snapshot_count",
                    )
                    .bind(("id", id))
                    .bind(("state", state_b64))
                    .bind(("sv", state_vector_b64))
                    .await?;
                let updated: Vec<CountRow> = q.take(0)?;
                Ok(updated
                    .into_iter()
                    .next()
                    .map(|r| r.snapshot_count)
                    .unwrap_or(1))
            }
        }
    }

    async fn append_checkpoint(
        &self,
        channel_id: &str,
        state_b64: String,
        label: Option<String>,
    ) -> Result<WhiteboardCheckpoint, AppError> {
        let now = chrono::Utc::now();
        let created: Option<WhiteboardCheckpoint> = self
            .db
            .create("whiteboard_checkpoint")
            .content(CreateCheckpointDb {
                channel: surrealdb::RecordId::from(("channel", channel_id)),
                state_b64,
                label,
                created_at: now,
            })
            .await?;
        let checkpoint = created
            .ok_or_else(|| AppError::Internal("Failed to insert checkpoint".into()))?;

        // Trim with a single atomic statement that keeps the newest
        // MAX_CHECKPOINTS rows and deletes the rest. The previous "SELECT
        // ids + per-row DELETE" approach silently failed (`DELETE $id` is
        // not valid SurrealQL syntax for a parameterized record ID), letting
        // checkpoints grow unbounded. Log on failure so trim regressions
        // surface in metrics instead of silently leaking disk.
        let trim_result = self
            .db
            .query(
                "DELETE whiteboard_checkpoint WHERE channel = $ch AND id NOT IN ( \
                    SELECT VALUE id FROM whiteboard_checkpoint \
                    WHERE channel = $ch \
                    ORDER BY created_at DESC LIMIT $cap \
                )",
            )
            .bind(("ch", surrealdb::RecordId::from(("channel", channel_id))))
            .bind(("cap", MAX_CHECKPOINTS))
            .await;
        if let Err(e) = trim_result {
            tracing::warn!(
                channel_id = %channel_id,
                error = %e,
                "Failed to trim old whiteboard checkpoints"
            );
        }

        Ok(checkpoint)
    }

    async fn list_checkpoints(
        &self,
        channel_id: &str,
    ) -> Result<Vec<WhiteboardCheckpoint>, AppError> {
        let mut q = self
            .db
            .query(
                "SELECT * FROM whiteboard_checkpoint WHERE channel = $ch \
                 ORDER BY created_at DESC LIMIT $limit",
            )
            .bind(("ch", surrealdb::RecordId::from(("channel", channel_id))))
            .bind(("limit", MAX_CHECKPOINTS))
            .await?;
        let rows: Vec<WhiteboardCheckpoint> = q.take(0)?;
        Ok(rows)
    }

    async fn find_checkpoint(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<WhiteboardCheckpoint>, AppError> {
        let row: Option<WhiteboardCheckpoint> = self
            .db
            .select(("whiteboard_checkpoint", checkpoint_id))
            .await?;
        Ok(row)
    }
}
