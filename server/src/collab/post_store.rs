use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::collab::doc::CollabDoc;
use crate::collab::resource::{
    ResourceKind, ResourceRef, ResourceStore, Snapshot, DEFAULT_PERSIST_DEBOUNCE,
};
use crate::repositories::post::PostRepo;

/// [`ResourceStore`] backed by [`PostRepo`]. Implements the Phase 2 collab
/// authorization (author + invited) and persistence (state_b64 +
/// state_vector_b64) semantics — kept identical so the generic refactor is
/// non-regressive.
pub struct PostStore {
    posts: Arc<dyn PostRepo>,
    persist_debounce: Duration,
}

impl PostStore {
    pub fn new(posts: Arc<dyn PostRepo>) -> Self {
        Self::with_debounce(posts, DEFAULT_PERSIST_DEBOUNCE)
    }

    /// Test/perf knob: override the debounce duration.
    pub fn with_debounce(posts: Arc<dyn PostRepo>, persist_debounce: Duration) -> Self {
        Self {
            posts,
            persist_debounce,
        }
    }
}

#[async_trait]
impl ResourceStore for PostStore {
    async fn load(&self, r: &ResourceRef) -> Result<Snapshot, String> {
        debug_assert!(matches!(r.kind, ResourceKind::Post));
        let post = self
            .posts
            .find_by_id(&r.id)
            .await
            .map_err(|e| format!("Failed to load post: {e}"))?
            .ok_or_else(|| "Post not found".to_string())?;
        Ok(Snapshot {
            state_b64: post.state_b64,
            state_vector_b64: post.state_vector_b64,
        })
    }

    async fn save(&self, r: &ResourceRef, snap: Snapshot) -> Result<(), String> {
        debug_assert!(matches!(r.kind, ResourceKind::Post));
        self.posts
            .save_snapshot(&r.id, snap.state_b64, snap.state_vector_b64)
            .await
            .map_err(|e| format!("save_snapshot failed: {e}"))
    }

    async fn authorize(&self, r: &ResourceRef, user_id: &str) -> Result<(), String> {
        debug_assert!(matches!(r.kind, ResourceKind::Post));
        let post = self
            .posts
            .find_by_id(&r.id)
            .await
            .map_err(|e| format!("Failed to load post: {e}"))?
            .ok_or_else(|| "Post not found".to_string())?;
        if post.published {
            return Err("Post is published — collab is closed".into());
        }
        let author_key = post.author.key().to_string();
        if author_key == user_id {
            return Ok(());
        }
        let invited = self
            .posts
            .is_invited(&r.id, user_id)
            .await
            .map_err(|e| format!("Auth check failed: {e}"))?;
        if invited {
            Ok(())
        } else {
            Err("Not authorized for this post".into())
        }
    }

    /// Publish — the actual DB write — lives in the `publish_post` HTTP
    /// handler so it can return the updated `Post` directly. `close` just
    /// notifies live subscribers and evicts the session; the on_close hook
    /// is intentionally a noop for posts.
    async fn on_close(&self, _r: &ResourceRef, _doc: &CollabDoc) -> Result<(), String> {
        Ok(())
    }

    fn persist_debounce(&self) -> Duration {
        self.persist_debounce
    }
}
