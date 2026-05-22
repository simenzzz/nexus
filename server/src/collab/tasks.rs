//! Long-lived background tasks for the collab manager: debounced
//! persistence, idle session eviction, and the small broadcast helper.
//! Lifted out of `mod.rs` so the manager file stays focused on the public
//! API surface.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::Mutex;

use super::resource::{ResourceKind, ResourceRef, ResourceStore, Snapshot};
use super::Session;

/// Debounced persistence loop. Wakes every `debounce`, flushes if dirty, and
/// exits when there's nothing left to save (the next edit will spawn a fresh
/// task via `apply_update`).
///
/// **Invariant**: this task is the sole owner of `persist_pending = true`
/// while running. It MUST clear that flag before returning so the next
/// dirty edit can spawn a fresh task. We use a guard struct so the flag is
/// cleared even on panic or unexpected early return.
pub(super) async fn persist_loop(
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
                return;
            }
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
pub(super) async fn sweeper_loop(
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

pub(super) async fn broadcast(session: &Session, skip_user: Option<&str>, payload: &str) {
    for sub in &session.subscribers {
        if matches!(skip_user, Some(u) if u == sub.user_id) {
            continue;
        }
        let _ = sub.tx.try_send(payload.to_string());
    }
}
