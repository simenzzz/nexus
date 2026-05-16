pub mod awareness;
pub mod doc;
pub mod post_store;
pub mod resource;
pub mod whiteboard_store;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};

use crate::repositories::post::PostRepo;

use awareness::Awareness;
use doc::CollabDoc;
use post_store::PostStore;
use resource::{
    awareness_message, closed_message, error_message, state_message, update_message,
    ResourceKind, ResourceRef, ResourceStore, Snapshot, DEFAULT_PERSIST_DEBOUNCE,
};

/// Hard cap on concurrent subscribers per document. Roadmap §2.4.2.
pub const MAX_COLLABORATORS: usize = 10;

/// How often the sweeper inspects sessions for idle eviction.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// A session with no subscribers and no edits within this window is dropped
/// from memory by the sweeper. Roadmap §2.4.3.
pub const IDLE_TTL: Duration = Duration::from_secs(60);

/// In-memory CRDT session. One per [`ResourceRef`] while the resource is
/// being edited; evicted lazily on last unsubscribe and defensively by the
/// sweeper if it goes idle.
struct Session {
    doc: CollabDoc,
    awareness: Awareness,
    subscribers: Vec<Subscriber>,
    /// True when the in-memory doc has edits not yet flushed to the store.
    dirty: bool,
    /// Timestamp of the most recent `apply_update`. Drives idle eviction.
    last_update_at: Instant,
    /// Single-flight flag: true while a debounced persist task is scheduled
    /// or running. The persist loop clears it before exiting; the next dirty
    /// edit will spawn a fresh task.
    persist_pending: bool,
}

struct Subscriber {
    user_id: String,
    tx: mpsc::Sender<String>,
}

/// Aborts a `JoinHandle` when dropped. Used so the sweeper task stops once
/// all `CollabManager` clones go out of scope (mainly relevant in tests).
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// In-memory CRDT manager. Generic over resource kind via [`ResourceStore`]
/// — Phase 2 wires a [`PostStore`], Phase 3 also wires a `WhiteboardStore`.
/// Holds active sessions keyed by [`ResourceRef`], dispatches WS messages,
/// debounces persistence, and runs an idle eviction sweeper.
#[derive(Clone)]
pub struct CollabManager {
    sessions: Arc<DashMap<ResourceRef, Arc<Mutex<Session>>>>,
    stores: Arc<HashMap<ResourceKind, Arc<dyn ResourceStore>>>,
    _sweeper: Arc<AbortOnDrop>,
}

impl CollabManager {
    /// Phase 2 convenience constructor — wires only the post store with the
    /// default debounce/sweep intervals.
    pub fn new(posts: Arc<dyn PostRepo>) -> Self {
        let mut stores: HashMap<ResourceKind, Arc<dyn ResourceStore>> = HashMap::new();
        stores.insert(ResourceKind::Post, Arc::new(PostStore::new(posts)));
        Self::with_stores(stores, SWEEP_INTERVAL, IDLE_TTL)
    }

    /// Wire multiple resource stores at once.
    pub fn with_stores(
        stores: HashMap<ResourceKind, Arc<dyn ResourceStore>>,
        sweep_interval: Duration,
        idle_ttl: Duration,
    ) -> Self {
        let sessions: Arc<DashMap<ResourceRef, Arc<Mutex<Session>>>> = Arc::new(DashMap::new());
        let stores = Arc::new(stores);
        let sweeper = {
            let sessions = sessions.clone();
            let stores = stores.clone();
            tokio::spawn(async move {
                sweeper_loop(sessions, stores, sweep_interval, idle_ttl).await;
            })
        };
        Self {
            sessions,
            stores,
            _sweeper: Arc::new(AbortOnDrop(sweeper)),
        }
    }

    /// Test-only constructor: post-only, with caller-supplied intervals.
    pub fn with_intervals(
        posts: Arc<dyn PostRepo>,
        persist_debounce: Duration,
        sweep_interval: Duration,
        idle_ttl: Duration,
    ) -> Self {
        let mut stores: HashMap<ResourceKind, Arc<dyn ResourceStore>> = HashMap::new();
        stores.insert(
            ResourceKind::Post,
            Arc::new(post_store::PostStore::with_debounce(posts, persist_debounce)),
        );
        Self::with_stores(stores, sweep_interval, idle_ttl)
    }

    fn store_for(&self, kind: ResourceKind) -> Result<Arc<dyn ResourceStore>, String> {
        self.stores
            .get(&kind)
            .cloned()
            .ok_or_else(|| format!("No store registered for {:?}", kind))
    }

    /// Subscribe `user_id` to a resource's session, hydrating from the store
    /// on cache miss. Sends a `*_state` message to `tx` on success.
    pub async fn subscribe(
        &self,
        r: &ResourceRef,
        user_id: &str,
        tx: mpsc::Sender<String>,
    ) -> Result<(), String> {
        let store = self.store_for(r.kind)?;
        store.authorize(r, user_id).await?;

        let session = match self.sessions.get(r).map(|s| s.clone()) {
            Some(s) => s,
            None => {
                let snap = store.load(r).await?;
                // Use the store's own cap so already-persisted large docs
                // (whiteboards up to 4 MB) can be rehydrated. The default
                // 256 KB cap would brick them.
                let doc = CollabDoc::from_snapshot_with_cap(&snap.state_b64, store.max_doc_bytes())
                    .map_err(|e| format!("Bad snapshot: {e}"))?;
                let s = Arc::new(Mutex::new(Session {
                    doc,
                    awareness: Awareness::new(),
                    subscribers: Vec::new(),
                    dirty: false,
                    last_update_at: Instant::now(),
                    persist_pending: false,
                }));
                self.sessions.insert(r.clone(), s.clone());
                s
            }
        };

        let mut s = session.lock().await;

        let already_subscribed = s.subscribers.iter().any(|sub| sub.user_id == user_id);
        if !already_subscribed && s.subscribers.len() >= MAX_COLLABORATORS {
            return Err(format!(
                "Session full (max {MAX_COLLABORATORS} collaborators)"
            ));
        }

        let snap = Snapshot {
            state_b64: s.doc.encode_state(),
            state_vector_b64: s.doc.encode_state_vector(),
        };
        let _ = tx.send(state_message(r, &snap).to_json()).await;

        if !s.awareness.is_empty() {
            let aw = awareness_message(r, s.awareness.snapshot()).to_json();
            let _ = tx.send(aw).await;
        }

        s.subscribers.retain(|sub| sub.user_id != user_id);
        s.subscribers.push(Subscriber {
            user_id: user_id.to_string(),
            tx,
        });
        Ok(())
    }

    /// Drop a user from a session. Public so the WS layer can clean up
    /// dangling subscriptions when a connection closes without sending an
    /// explicit unsubscribe (tab close, network drop).
    pub async fn unsubscribe(&self, r: &ResourceRef, user_id: &str) {
        let Some(session) = self.sessions.get(r).map(|s| s.clone()) else {
            return;
        };

        let empty = {
            let mut s = session.lock().await;
            s.subscribers.retain(|sub| sub.user_id != user_id);
            s.awareness.remove(user_id);

            let payload = awareness_message(r, s.awareness.snapshot()).to_json();
            broadcast(&s, None, &payload).await;
            s.subscribers.is_empty()
        };

        if empty {
            self.flush_now(r).await;
            self.sessions.remove(r);
        }
    }

    /// Apply a remote update from `from_user` and fan it out to peers.
    pub async fn apply_update(
        &self,
        r: &ResourceRef,
        from_user: &str,
        update_b64: &str,
    ) -> Result<(), String> {
        let store = self.store_for(r.kind)?;
        let max_update = store.max_update_bytes();
        let max_doc = store.max_doc_bytes();

        let session = self
            .sessions
            .get(r)
            .ok_or_else(|| "Not subscribed".to_string())?
            .clone();

        let need_spawn = {
            let mut s = session.lock().await;
            s.doc
                .apply_update_with_cap(update_b64, max_update)
                .map_err(|e| format!("Apply failed: {e}"))?;

            // Post-merge ceiling: reject the update if the merged doc would
            // grow past the per-resource cap. The update has already been
            // applied to the local doc — but since no other clients see it
            // until broadcast below, returning the error here is sufficient
            // protection for *future* peers' hydration size. Tombstones we
            // can't undo are acceptable (yrs converges anyway).
            if s.doc.encoded_state_len() > max_doc {
                return Err(format!("Document exceeds {max_doc}-byte limit"));
            }

            let payload = update_message(r, update_b64.to_string(), from_user.to_string()).to_json();
            broadcast(&s, Some(from_user), &payload).await;

            s.dirty = true;
            s.last_update_at = Instant::now();
            if s.persist_pending {
                false
            } else {
                s.persist_pending = true;
                true
            }
        };

        if need_spawn {
            let store = self.store_for(r.kind)?;
            let debounce = store.persist_debounce();
            let session_for_task = session.clone();
            let r_owned = r.clone();
            tokio::spawn(async move {
                persist_loop(session_for_task, store, r_owned, debounce).await;
            });
        }
        Ok(())
    }

    /// Update awareness state for a user and broadcast the new snapshot.
    pub async fn update_awareness(&self, r: &ResourceRef, user_id: &str, state: Value) {
        if let Some(session) = self.sessions.get(r) {
            let session = session.clone();
            let mut s = session.lock().await;
            s.awareness.update(user_id.to_string(), state);
            let payload = awareness_message(r, s.awareness.snapshot()).to_json();
            broadcast(&s, None, &payload).await;
        }
    }

    /// Tear down a session — flush pending bytes, run the store's `on_close`
    /// hook, notify subscribers, evict the doc. No-op if no session is
    /// cached (nothing to tear down; resource-level finalization is the
    /// caller's responsibility).
    ///
    /// The broadcast + eviction always happens, even when `on_close` errors,
    /// so a failed finalizer can't strand the session in memory. The
    /// `on_close` error is returned to the caller so they can decide whether
    /// the partial teardown is fatal.
    pub async fn close(&self, r: &ResourceRef, reason: &str) -> Result<(), String> {
        let store = self.store_for(r.kind)?;
        self.flush_now(r).await;

        let Some(session) = self.sessions.get(r).map(|s| s.clone()) else {
            return Ok(());
        };
        let on_close_err = {
            let s = session.lock().await;
            // Run the store hook first, but record the error rather than
            // returning early — we still want to broadcast and evict.
            let err = store.on_close(r, &s.doc).await.err();
            let payload = closed_message(r, reason.to_string()).to_json();
            broadcast(&s, None, &payload).await;
            err
        };
        self.sessions.remove(r);
        match on_close_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Helper for the WS layer: build an error ServerMessage scoped to the
    /// resource and emit it via `tx`. Falls back to silent drop on send fail.
    pub async fn send_error(
        tx: &mpsc::Sender<String>,
        r: &ResourceRef,
        code: &str,
        message: &str,
    ) {
        let _ = tx
            .try_send(error_message(r, code.to_string(), message.to_string()).to_json());
    }

    /// Force an immediate save and clear the dirty flag. Safe to call when
    /// nothing is dirty (returns without touching the store). Used at session
    /// teardown points to bound data loss.
    async fn flush_now(&self, r: &ResourceRef) {
        let Some(session) = self.sessions.get(r).map(|s| s.clone()) else {
            return;
        };
        let payload = {
            let mut s = session.lock().await;
            if !s.dirty {
                return;
            }
            s.dirty = false;
            Some(Snapshot {
                state_b64: s.doc.encode_state(),
                state_vector_b64: s.doc.encode_state_vector(),
            })
        };
        if let Some(snap) = payload {
            if let Ok(store) = self.store_for(r.kind) {
                if let Err(e) = store.save(r, snap).await {
                    tracing::warn!(resource = ?r, error = %e, "Flush failed");
                    if let Some(session) = self.sessions.get(r).map(|s| s.clone()) {
                        session.lock().await.dirty = true;
                    }
                }
            }
        }
    }
}

/// Debounced persistence loop. Wakes every `debounce`, flushes if dirty, and
/// exits when there's nothing left to save (the next edit will spawn a fresh
/// task via `apply_update`).
///
/// **Invariant**: this task is the sole owner of `persist_pending = true`
/// while running. It MUST clear that flag before returning so the next
/// dirty edit can spawn a fresh task. We use a guard struct so the flag is
/// cleared even on panic or unexpected early return — without it, a panic
/// would strand the flag at `true` forever and silently drop all subsequent
/// edits.
async fn persist_loop(
    session: Arc<Mutex<Session>>,
    store: Arc<dyn ResourceStore>,
    r: ResourceRef,
    debounce: Duration,
) {
    /// Cleared in `Drop` — guarantees `persist_pending` is reset under all
    /// exit paths (normal return, panic, future cancel).
    struct ClearPending(Arc<Mutex<Session>>);
    impl Drop for ClearPending {
        fn drop(&mut self) {
            // Best-effort clear: if the lock is contended in a panic-unwind
            // path the next dirty edit will hit `persist_pending = true` and
            // skip spawning, but on the *next* save attempt the loop will
            // pick up the dirty flag and persist. Worst case = one debounce
            // window of staleness, never permanent loss.
            if let Ok(mut s) = self.0.try_lock() {
                s.persist_pending = false;
            } else {
                let s = self.0.clone();
                tokio::spawn(async move {
                    s.lock().await.persist_pending = false;
                });
            }
        }
    }
    let _guard = ClearPending(session.clone());

    loop {
        tokio::time::sleep(debounce).await;

        let snap = {
            let mut s = session.lock().await;
            if !s.dirty {
                // Guard clears `persist_pending` on return.
                return;
            }
            // Optimistically clear dirty — re-set below on save failure.
            // Edits landing between here and save-completion will re-flip
            // dirty to true; the loop catches them on the next iteration.
            s.dirty = false;
            Snapshot {
                state_b64: s.doc.encode_state(),
                state_vector_b64: s.doc.encode_state_vector(),
            }
        };

        if let Err(e) = store.save(&r, snap).await {
            tracing::warn!(resource = ?r, error = %e, "Snapshot persist failed; retrying");
            session.lock().await.dirty = true;
        }
    }
}

/// Idle-eviction sweeper. Drops sessions with no subscribers that haven't
/// seen an edit in `idle_ttl`.
async fn sweeper_loop(
    sessions: Arc<DashMap<ResourceRef, Arc<Mutex<Session>>>>,
    stores: Arc<HashMap<ResourceKind, Arc<dyn ResourceStore>>>,
    sweep_interval: Duration,
    idle_ttl: Duration,
) {
    loop {
        tokio::time::sleep(sweep_interval).await;
        let now = Instant::now();
        let mut to_evict: Vec<ResourceRef> = Vec::new();

        for entry in sessions.iter() {
            let key = entry.key().clone();
            let session = entry.value().clone();
            drop(entry);
            let s = match session.try_lock() {
                Ok(g) => g,
                Err(_) => continue,
            };
            if s.subscribers.is_empty() && now.duration_since(s.last_update_at) > idle_ttl {
                to_evict.push(key);
            }
        }

        for key in to_evict {
            let Some(session) = sessions.get(&key).map(|s| s.clone()) else {
                continue;
            };
            let snap = {
                let mut s = session.lock().await;
                if s.dirty {
                    s.dirty = false;
                    Some(Snapshot {
                        state_b64: s.doc.encode_state(),
                        state_vector_b64: s.doc.encode_state_vector(),
                    })
                } else {
                    None
                }
            };
            if let Some(snap) = snap {
                let store = match stores.get(&key.kind) {
                    Some(s) => s.clone(),
                    None => {
                        tracing::warn!(resource = ?key, "Sweeper: no store registered");
                        continue;
                    }
                };
                if let Err(e) = store.save(&key, snap).await {
                    tracing::warn!(resource = ?key, error = %e, "Sweeper flush failed");
                    session.lock().await.dirty = true;
                    continue;
                }
            }
            sessions.remove(&key);
        }
    }
}

async fn broadcast(session: &Session, skip_user: Option<&str>, payload: &str) {
    for sub in &session.subscribers {
        if matches!(skip_user, Some(u) if u == sub.user_id) {
            continue;
        }
        let _ = sub.tx.try_send(payload.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::post::Post;
    use crate::repositories::post::MockPostRepo;
    use serde_json::json;
    use yrs::{ReadTxn, StateVector, Text, Transact};

    fn post_ref() -> ResourceRef {
        ResourceRef::post("p1")
    }

    fn fake_post(author: &str, published: bool) -> Post {
        fake_post_with_state(author, published, String::new())
    }

    fn fake_post_with_state(author: &str, published: bool, state_b64: String) -> Post {
        Post {
            id: Some(surrealdb::RecordId::from(("post", "p1"))),
            author: surrealdb::RecordId::from(("user", author)),
            title: "draft".into(),
            state_b64,
            state_vector_b64: String::new(),
            published,
            published_content: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn encoded_state_with_text(text: &str) -> String {
        let doc = yrs::Doc::new();
        let t = doc.get_or_insert_text(doc::TEXT_ROOT);
        let mut txn = doc.transact_mut();
        t.insert(&mut txn, 0, text);
        let bytes = txn.encode_state_as_update_v1(&StateVector::default());
        drop(txn);
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn encoded_update_inserting(text: &str) -> String {
        encoded_state_with_text(text)
    }

    fn fast_intervals() -> CollabManager {
        let posts = MockPostRepo::new();
        CollabManager::with_intervals(
            Arc::new(posts),
            Duration::from_millis(20),
            Duration::from_millis(25),
            Duration::from_millis(50),
        )
    }

    fn fast_intervals_with(posts: Arc<dyn PostRepo>) -> CollabManager {
        CollabManager::with_intervals(
            posts,
            Duration::from_millis(20),
            Duration::from_millis(25),
            Duration::from_millis(50),
        )
    }

    #[tokio::test]
    async fn subscribe_rejects_non_collaborator() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("author", false))));
        posts.expect_is_invited().returning(|_, _| Ok(false));

        let manager = CollabManager::new(Arc::new(posts));
        let (tx, _rx) = mpsc::channel(8);

        let result = manager.subscribe(&post_ref(), "stranger", tx).await;
        assert!(result.is_err());
        assert!(manager.sessions.get(&post_ref()).is_none());
    }

    #[tokio::test]
    async fn subscribe_rejects_published_post() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("author", true))));

        let manager = CollabManager::new(Arc::new(posts));
        let (tx, _rx) = mpsc::channel(8);

        let result = manager.subscribe(&post_ref(), "author", tx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unsubscribe_evicts_session_when_last_user_leaves() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("u1", false))));
        posts.expect_is_invited().never();

        let manager = CollabManager::new(Arc::new(posts));
        let (tx, _rx) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx)
            .await
            .expect("subscribe");
        assert!(manager.sessions.get(&post_ref()).is_some());

        manager.unsubscribe(&post_ref(), "u1").await;
        assert!(
            manager.sessions.get(&post_ref()).is_none(),
            "session should be evicted when the last subscriber leaves"
        );
    }

    #[tokio::test]
    async fn close_removes_session_and_notifies() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("u1", false))));
        posts.expect_is_invited().never();

        let manager = CollabManager::new(Arc::new(posts));
        let (tx, mut rx) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx)
            .await
            .expect("subscribe");

        let _ = rx.recv().await;

        manager
            .close(&post_ref(), "published")
            .await
            .expect("close should succeed");
        assert!(manager.sessions.get(&post_ref()).is_none());

        let notice = rx.recv().await.expect("subscriber should be notified");
        assert!(notice.contains("collab_closed"));
        assert!(notice.contains("published"));
    }

    #[tokio::test]
    async fn update_broadcast_reaches_other_subscriber() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("u1", false))));
        posts.expect_is_invited().returning(|_, _| Ok(true));
        posts.expect_save_snapshot().returning(|_, _, _| Ok(()));

        let manager = CollabManager::new(Arc::new(posts));

        let (tx_a, mut rx_a) = mpsc::channel(8);
        let (tx_b, mut rx_b) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx_a)
            .await
            .expect("u1 subscribe");
        manager
            .subscribe(&post_ref(), "u2", tx_b)
            .await
            .expect("u2 subscribe");

        let _ = rx_a.recv().await;
        let _ = rx_b.recv().await;

        let update = encoded_update_inserting("hello");
        manager
            .apply_update(&post_ref(), "u1", &update)
            .await
            .expect("apply_update");

        let received = rx_b.recv().await.expect("u2 receives update");
        assert!(received.contains("collab_update"));
        assert!(received.contains("\"from_user\":\"u1\""));
        assert!(received.contains(&update));

        let echoed = rx_a.try_recv();
        assert!(
            echoed.is_err(),
            "sender should not receive their own update echo, got: {echoed:?}"
        );
    }

    #[tokio::test]
    async fn awareness_broadcast_reaches_other_subscriber() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("u1", false))));
        posts.expect_is_invited().returning(|_, _| Ok(true));

        let manager = CollabManager::new(Arc::new(posts));

        let (tx_a, mut rx_a) = mpsc::channel(8);
        let (tx_b, mut rx_b) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx_a)
            .await
            .expect("u1 subscribe");
        manager
            .subscribe(&post_ref(), "u2", tx_b)
            .await
            .expect("u2 subscribe");

        let _ = rx_a.recv().await;
        let _ = rx_b.recv().await;
        while rx_a.try_recv().is_ok() {}
        while rx_b.try_recv().is_ok() {}

        manager
            .update_awareness(&post_ref(), "u1", json!({ "cursor": 7 }))
            .await;

        let msg = rx_b.recv().await.expect("u2 receives awareness");
        assert!(msg.contains("awareness_state"));
        assert!(msg.contains("\"u1\""));
        assert!(msg.contains("\"cursor\":7"));
    }

    #[tokio::test]
    async fn state_replays_to_resubscribed_user() {
        let seeded = encoded_state_with_text("durable");

        let mut posts = MockPostRepo::new();
        let seeded_clone = seeded.clone();
        posts.expect_find_by_id().returning(move |_| {
            Ok(Some(fake_post_with_state("u1", false, seeded_clone.clone())))
        });
        posts.expect_is_invited().returning(|_, _| Ok(true));
        posts.expect_save_snapshot().returning(|_, _, _| Ok(()));

        let manager = CollabManager::new(Arc::new(posts));

        let (tx1, mut rx1) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u2", tx1)
            .await
            .expect("subscribe");
        let initial = rx1.recv().await.expect("collab_state");
        assert!(initial.contains("collab_state"));
        assert!(initial.contains(&seeded));

        manager.unsubscribe(&post_ref(), "u2").await;
        let (tx2, mut rx2) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u2", tx2)
            .await
            .expect("resubscribe");
        let replay = rx2.recv().await.expect("collab_state on resubscribe");
        assert!(replay.contains("collab_state"));
        assert!(replay.contains(&seeded));
    }

    #[tokio::test]
    async fn subscribe_rejects_when_session_full() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("author", false))));
        posts.expect_is_invited().returning(|_, _| Ok(true));

        let manager = CollabManager::new(Arc::new(posts));

        for i in 0..MAX_COLLABORATORS {
            let (tx, _rx) = mpsc::channel(8);
            std::mem::forget(_rx);
            manager
                .subscribe(&post_ref(), &format!("u{i}"), tx)
                .await
                .expect("under cap");
        }

        let (tx, _rx) = mpsc::channel(8);
        let err = manager
            .subscribe(&post_ref(), "overflow", tx)
            .await
            .expect_err("should reject when full");
        assert!(err.contains("Session full"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn evicted_session_rehydrates_from_snapshot() {
        let saved: Arc<std::sync::Mutex<String>> =
            Arc::new(std::sync::Mutex::new(String::new()));

        let mut posts = MockPostRepo::new();
        let saved_for_find = saved.clone();
        posts.expect_find_by_id().returning(move |_| {
            let state = saved_for_find.lock().unwrap().clone();
            Ok(Some(fake_post_with_state("u1", false, state)))
        });
        posts.expect_is_invited().returning(|_, _| Ok(true));
        let saved_for_save = saved.clone();
        posts
            .expect_save_snapshot()
            .returning(move |_, state, _| {
                *saved_for_save.lock().unwrap() = state;
                Ok(())
            });

        let manager = CollabManager::with_intervals(
            Arc::new(posts),
            Duration::from_millis(20),
            Duration::from_millis(500),
            Duration::from_secs(60),
        );

        let (tx, mut rx) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx)
            .await
            .expect("subscribe");
        let _ = rx.recv().await;

        let update = encoded_update_inserting("evict-me");
        manager
            .apply_update(&post_ref(), "u1", &update)
            .await
            .expect("apply");

        tokio::time::sleep(Duration::from_millis(80)).await;

        manager.unsubscribe(&post_ref(), "u1").await;
        assert!(
            manager.sessions.get(&post_ref()).is_none(),
            "session evicted"
        );
        assert!(
            !saved.lock().unwrap().is_empty(),
            "snapshot should have been persisted"
        );

        let (tx2, mut rx2) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx2)
            .await
            .expect("resubscribe");
        let hello = rx2.recv().await.expect("collab_state");
        assert!(hello.contains("collab_state"));
        let saved_b64 = saved.lock().unwrap().clone();
        assert!(
            hello.contains(&saved_b64),
            "expected state to embed saved snapshot {saved_b64}, got {hello}"
        );

        let restored = doc::CollabDoc::from_snapshot(&saved_b64).expect("restore");
        assert_eq!(restored.text(), "evict-me");
    }

    #[tokio::test]
    async fn idle_session_evicted_after_ttl() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("u1", false))));
        posts.expect_is_invited().never();

        let manager = fast_intervals_with(Arc::new(posts));

        let (tx, _rx) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx)
            .await
            .expect("subscribe");

        manager.unsubscribe(&post_ref(), "u1").await;
        assert!(
            manager.sessions.get(&post_ref()).is_none(),
            "unsubscribe evicts"
        );

        let zombie = ResourceRef::post("zombie");
        let stale = Arc::new(Mutex::new(Session {
            doc: doc::CollabDoc::new(),
            awareness: Awareness::new(),
            subscribers: Vec::new(),
            dirty: false,
            last_update_at: Instant::now() - Duration::from_secs(3600),
            persist_pending: false,
        }));
        manager.sessions.insert(zombie.clone(), stale);

        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(
            manager.sessions.get(&zombie).is_none(),
            "sweeper should have evicted the idle session"
        );
    }

    #[tokio::test]
    async fn fast_intervals_constructor_smoke() {
        let _m = fast_intervals();
    }

    // ---------- §3.1 verification: post + whiteboard coexistence ----------

    use crate::collab::whiteboard_store::WhiteboardStore;
    use crate::models::channel::{Channel, ChannelType};
    use crate::repositories::channel::MockChannelRepo;
    use crate::repositories::server::MockServerRepo;
    use crate::repositories::whiteboard::MockWhiteboardRepo;

    fn wb_ref() -> ResourceRef {
        ResourceRef::whiteboard("c1")
    }

    fn fake_channel(id: &str, server_id: &str, kind: ChannelType) -> Channel {
        Channel {
            id: Some(surrealdb::RecordId::from(("channel", id))),
            name: "wb".into(),
            channel_type: kind,
            server: surrealdb::RecordId::from(("server", server_id)),
            created_at: None,
        }
    }

    fn manager_with_both(
        posts: Arc<dyn PostRepo>,
        whiteboards: Arc<dyn crate::repositories::whiteboard::WhiteboardRepo>,
        channels: Arc<dyn crate::repositories::channel::ChannelRepo>,
        servers: Arc<dyn crate::repositories::server::ServerRepo>,
    ) -> CollabManager {
        let mut stores: HashMap<ResourceKind, Arc<dyn ResourceStore>> = HashMap::new();
        stores.insert(
            ResourceKind::Post,
            Arc::new(post_store::PostStore::with_debounce(
                posts,
                Duration::from_millis(20),
            )),
        );
        stores.insert(
            ResourceKind::Whiteboard,
            Arc::new(WhiteboardStore::with_intervals(
                whiteboards,
                channels,
                servers,
                Duration::from_millis(20),
                0,
            )),
        );
        CollabManager::with_stores(
            stores,
            Duration::from_millis(500),
            Duration::from_secs(60),
        )
    }

    #[tokio::test]
    async fn posts_and_whiteboards_coexist_in_one_manager() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_post("u1", false))));
        posts.expect_is_invited().never();
        posts.expect_save_snapshot().returning(|_, _, _| Ok(()));

        let mut wbs = MockWhiteboardRepo::new();
        wbs.expect_find_by_channel().returning(|_| Ok(None));
        wbs.expect_upsert_snapshot().returning(|_, _, _| Ok(1));

        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_channel("c1", "s1", ChannelType::Whiteboard))));

        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));

        let manager = manager_with_both(
            Arc::new(posts),
            Arc::new(wbs),
            Arc::new(chans),
            Arc::new(servers),
        );

        // Subscribe to a post — must succeed and seed a session.
        let (tx_post, mut rx_post) = mpsc::channel(8);
        manager
            .subscribe(&post_ref(), "u1", tx_post)
            .await
            .expect("post subscribe");
        let hello_post = rx_post.recv().await.expect("post collab_state");
        assert!(hello_post.contains("collab_state"));

        // Subscribe to a whiteboard with the same user — must succeed and
        // seed a *separate* session keyed by ResourceRef::Whiteboard.
        let (tx_wb, mut rx_wb) = mpsc::channel(8);
        manager
            .subscribe(&wb_ref(), "u1", tx_wb)
            .await
            .expect("whiteboard subscribe");
        let hello_wb = rx_wb.recv().await.expect("whiteboard state");
        assert!(hello_wb.contains("whiteboard_state"));

        // Both sessions live in the same DashMap, keyed independently.
        assert!(manager.sessions.get(&post_ref()).is_some());
        assert!(manager.sessions.get(&wb_ref()).is_some());
    }

    #[tokio::test]
    async fn whiteboard_authorize_rejects_non_member() {
        let posts = MockPostRepo::new();
        let wbs = MockWhiteboardRepo::new();
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(fake_channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(false));

        let manager = manager_with_both(
            Arc::new(posts),
            Arc::new(wbs),
            Arc::new(chans),
            Arc::new(servers),
        );

        let (tx, _rx) = mpsc::channel(8);
        let err = manager
            .subscribe(&wb_ref(), "stranger", tx)
            .await
            .expect_err("non-member must be rejected");
        assert!(err.contains("Not a member"));
        // No session was created.
        assert!(manager.sessions.get(&wb_ref()).is_none());
    }
}
