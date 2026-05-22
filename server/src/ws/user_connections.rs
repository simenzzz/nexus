use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;

/// A single WebSocket connection identified by a unique connection ID.
struct Connection {
    id: String,
    sender: mpsc::Sender<String>,
}

/// Registry mapping user IDs to their active WebSocket sender handles.
/// Used for presence fan-out (notifying friends when a user comes online/goes offline).
#[derive(Clone, Default)]
pub struct UserConnectionRegistry {
    inner: Arc<DashMap<String, Vec<Connection>>>,
}

impl UserConnectionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Register a sender for a user. Called when a WS connection is established.
    /// Returns a connection ID for later unregistration.
    pub fn register(&self, user_id: &str, conn_id: String, sender: mpsc::Sender<String>) {
        self.inner
            .entry(user_id.to_string())
            .or_default()
            .push(Connection {
                id: conn_id,
                sender,
            });
    }

    /// Unregister a connection by ID. Called when a WS connection closes.
    /// Removes the user entry entirely if no connections remain.
    pub fn unregister(&self, user_id: &str, conn_id: &str) {
        let should_remove = {
            if let Some(mut conns) = self.inner.get_mut(user_id) {
                conns.retain(|c| c.id != conn_id);
                conns.is_empty()
            } else {
                false
            }
        };
        if should_remove {
            self.inner.remove(user_id);
        }
    }

    /// Check if a user has any active connections.
    pub fn is_online(&self, user_id: &str) -> bool {
        self.inner.get(user_id).map_or(false, |s| !s.is_empty())
    }

    /// Send a message to all connections of a specific user.
    /// Silently skips closed channels.
    pub async fn send_to_user(&self, user_id: &str, message: String) {
        if let Some(conns) = self.inner.get(user_id) {
            for conn in conns.iter() {
                let _ = conn.sender.send(message.clone()).await;
            }
        }
    }

    /// Send a message to all connections of multiple users.
    pub async fn send_to_users(&self, user_ids: &[String], message: String) {
        for user_id in user_ids {
            self.send_to_user(user_id, message.clone()).await;
        }
    }
}
