use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::collab::doc::CollabDoc;
use crate::collab::resource::{ResourceKind, ResourceRef, ResourceStore, Snapshot};
use crate::models::whiteboard::{WHITEBOARD_MAX_DOC_BYTES, WHITEBOARD_MAX_UPDATE_BYTES};
use crate::repositories::channel::ChannelRepo;
use crate::repositories::server::ServerRepo;
use crate::repositories::whiteboard::WhiteboardRepo;

/// Snapshot interval for whiteboards. Persisting every 30 s bounds data loss
/// on crash while keeping write amplification well below per-keystroke for an
/// active collaborative drawing session. Roadmap §3.5.
pub const WHITEBOARD_PERSIST_DEBOUNCE: Duration = Duration::from_secs(30);

/// Write a checkpoint every Nth snapshot. ~10 snapshots × 30 s debounce ≈ a
/// fresh checkpoint every 5 minutes of active editing. Cap at
/// [`MAX_CHECKPOINTS`](crate::models::whiteboard::MAX_CHECKPOINTS); oldest
/// trimmed by the repo.
pub const CHECKPOINT_EVERY_N_SNAPSHOTS: u64 = 10;

/// [`ResourceStore`] for whiteboard channels. Wires authorization (server
/// membership), per-channel persistence, and periodic checkpoint writes.
pub struct WhiteboardStore {
    whiteboards: Arc<dyn WhiteboardRepo>,
    channels: Arc<dyn ChannelRepo>,
    servers: Arc<dyn ServerRepo>,
    persist_debounce: Duration,
    checkpoint_every_n: u64,
}

impl WhiteboardStore {
    pub fn new(
        whiteboards: Arc<dyn WhiteboardRepo>,
        channels: Arc<dyn ChannelRepo>,
        servers: Arc<dyn ServerRepo>,
    ) -> Self {
        Self {
            whiteboards,
            channels,
            servers,
            persist_debounce: WHITEBOARD_PERSIST_DEBOUNCE,
            checkpoint_every_n: CHECKPOINT_EVERY_N_SNAPSHOTS,
        }
    }

    pub fn with_intervals(
        whiteboards: Arc<dyn WhiteboardRepo>,
        channels: Arc<dyn ChannelRepo>,
        servers: Arc<dyn ServerRepo>,
        persist_debounce: Duration,
        checkpoint_every_n: u64,
    ) -> Self {
        Self {
            whiteboards,
            channels,
            servers,
            persist_debounce,
            checkpoint_every_n,
        }
    }
}

#[async_trait]
impl ResourceStore for WhiteboardStore {
    async fn load(&self, r: &ResourceRef) -> Result<Snapshot, String> {
        debug_assert!(matches!(r.kind, ResourceKind::Whiteboard));
        // Auto-create on first edit: missing row → empty snapshot. The first
        // `save` will then upsert and create the row.
        let Some(wb) = self
            .whiteboards
            .find_by_channel(&r.id)
            .await
            .map_err(|e| format!("Failed to load whiteboard: {e}"))?
        else {
            return Ok(Snapshot::empty());
        };
        Ok(Snapshot {
            state_b64: wb.state_b64,
            state_vector_b64: wb.state_vector_b64,
        })
    }

    async fn save(&self, r: &ResourceRef, snap: Snapshot) -> Result<(), String> {
        debug_assert!(matches!(r.kind, ResourceKind::Whiteboard));
        // Size cap is enforced upstream in CollabManager::apply_update via
        // ResourceStore::max_doc_bytes — by the time we save, the doc is
        // already within bounds.
        let state_clone = snap.state_b64.clone();
        let count = self
            .whiteboards
            .upsert_snapshot(&r.id, snap.state_b64, snap.state_vector_b64)
            .await
            .map_err(|e| format!("upsert_snapshot failed: {e}"))?;

        // Best-effort checkpoint write every Nth snapshot. Failure here must
        // not abort the primary save — the snapshot is already durable.
        if self.checkpoint_every_n > 0 && count % self.checkpoint_every_n == 0 {
            if let Err(e) = self
                .whiteboards
                .append_checkpoint(&r.id, state_clone, None)
                .await
            {
                tracing::warn!(channel = %r.id, error = %e, "checkpoint write failed");
            }
        }
        Ok(())
    }

    async fn authorize(&self, r: &ResourceRef, user_id: &str) -> Result<(), String> {
        debug_assert!(matches!(r.kind, ResourceKind::Whiteboard));
        let channel = self
            .channels
            .find_by_id(&r.id)
            .await
            .map_err(|e| format!("Failed to load channel: {e}"))?
            .ok_or_else(|| "Channel not found".to_string())?;

        // Only whiteboard-typed channels accept collab subscriptions on this
        // resource kind. Belt-and-braces against a client sending a
        // whiteboard_subscribe with a text/voice channel ID.
        if !matches!(
            channel.channel_type,
            crate::models::channel::ChannelType::Whiteboard
        ) {
            return Err("Channel is not a whiteboard".into());
        }

        let server_key = channel.server.key().to_string();
        let is_member = self
            .servers
            .is_member(&server_key, user_id)
            .await
            .map_err(|e| format!("Membership check failed: {e}"))?;
        if !is_member {
            return Err("Not a member of this server".into());
        }
        Ok(())
    }

    /// Whiteboards are persistent — they never "publish" or close. The hook
    /// is only triggered on checkpoint restore, where the manager-level
    /// broadcast (handled by `close`) is the entire effect.
    async fn on_close(&self, _r: &ResourceRef, _doc: &CollabDoc) -> Result<(), String> {
        Ok(())
    }

    fn persist_debounce(&self) -> Duration {
        self.persist_debounce
    }

    fn max_update_bytes(&self) -> usize {
        WHITEBOARD_MAX_UPDATE_BYTES
    }

    fn max_doc_bytes(&self) -> usize {
        WHITEBOARD_MAX_DOC_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::channel::{Channel, ChannelType};
    use crate::repositories::channel::MockChannelRepo;
    use crate::repositories::server::MockServerRepo;
    use crate::repositories::whiteboard::MockWhiteboardRepo;

    fn channel(id: &str, server_id: &str, kind: ChannelType) -> Channel {
        Channel {
            id: Some(surrealdb::RecordId::from(("channel", id))),
            name: "wb".into(),
            channel_type: kind,
            server: surrealdb::RecordId::from(("server", server_id)),
            created_at: None,
        }
    }

    fn store(
        whiteboards: MockWhiteboardRepo,
        channels: MockChannelRepo,
        servers: MockServerRepo,
    ) -> WhiteboardStore {
        WhiteboardStore::new(Arc::new(whiteboards), Arc::new(channels), Arc::new(servers))
    }

    #[tokio::test]
    async fn authorize_rejects_non_whiteboard_channel() {
        let wbs = MockWhiteboardRepo::new();
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Text))));
        let servers = MockServerRepo::new();

        let s = store(wbs, chans, servers);
        let err = s
            .authorize(&ResourceRef::whiteboard("c1"), "u1")
            .await
            .expect_err("non-whiteboard channel must be rejected");
        assert!(err.contains("not a whiteboard"));
    }

    #[tokio::test]
    async fn authorize_rejects_non_server_member() {
        let wbs = MockWhiteboardRepo::new();
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(false));

        let s = store(wbs, chans, servers);
        let err = s
            .authorize(&ResourceRef::whiteboard("c1"), "stranger")
            .await
            .expect_err("non-member must be rejected");
        assert!(err.contains("Not a member"));
    }

    #[tokio::test]
    async fn authorize_allows_server_member_on_whiteboard() {
        let wbs = MockWhiteboardRepo::new();
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));

        let s = store(wbs, chans, servers);
        s.authorize(&ResourceRef::whiteboard("c1"), "u1")
            .await
            .expect("server member must be allowed");
    }

    #[tokio::test]
    async fn load_returns_empty_for_uninitialized_channel() {
        let mut wbs = MockWhiteboardRepo::new();
        wbs.expect_find_by_channel().returning(|_| Ok(None));
        let chans = MockChannelRepo::new();
        let servers = MockServerRepo::new();

        let s = store(wbs, chans, servers);
        let snap = s.load(&ResourceRef::whiteboard("c1")).await.expect("load");
        assert_eq!(snap.state_b64, "");
        assert_eq!(snap.state_vector_b64, "");
    }
}
