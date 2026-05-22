use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::collab::doc::CollabDoc;
use crate::ws::protocol::ServerMessage;

/// Default debounce between dirty flag and snapshot persist. Stores can
/// override via [`ResourceStore::persist_debounce`].
pub const DEFAULT_PERSIST_DEBOUNCE: Duration = Duration::from_secs(2);

/// Discriminates the resource kinds the [`CollabManager`](super::CollabManager)
/// can host. New CRDT-backed resources add a variant here and a matching
/// [`ResourceStore`] impl.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ResourceKind {
    Post,
    Whiteboard,
}

/// Stable key for a single live collab session.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub id: String,
}

impl ResourceRef {
    pub fn post(id: impl Into<String>) -> Self {
        Self {
            kind: ResourceKind::Post,
            id: id.into(),
        }
    }

    pub fn whiteboard(id: impl Into<String>) -> Self {
        Self {
            kind: ResourceKind::Whiteboard,
            id: id.into(),
        }
    }
}

/// Persisted CRDT bytes. Empty strings represent a fresh, never-saved document.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub state_b64: String,
    pub state_vector_b64: String,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Per-resource-kind backend the manager delegates to. Encapsulates
/// authorization and persistence so the session/dispatch core stays generic.
#[async_trait]
pub trait ResourceStore: Send + Sync {
    /// Load the current snapshot. Should return an empty snapshot for
    /// resources that are auto-created on first edit (e.g., whiteboards);
    /// should error for resources that must already exist (e.g., posts).
    async fn load(&self, r: &ResourceRef) -> Result<Snapshot, String>;

    /// Persist the snapshot. Called from the debounced persist loop.
    async fn save(&self, r: &ResourceRef, snap: Snapshot) -> Result<(), String>;

    /// Authorize a user to subscribe. Display-safe error string on rejection.
    async fn authorize(&self, r: &ResourceRef, user_id: &str) -> Result<(), String>;

    /// Hook fired by [`CollabManager::close`]. Receives the final in-memory
    /// doc so the store can finalize content (e.g., extract text + publish).
    /// Default is a no-op (whiteboards never "close").
    async fn on_close(&self, _r: &ResourceRef, _doc: &CollabDoc) -> Result<(), String> {
        Ok(())
    }

    /// Debounce window between a dirty-flag set and the next persist call.
    fn persist_debounce(&self) -> Duration {
        DEFAULT_PERSIST_DEBOUNCE
    }

    /// Maximum byte length of a single decoded Yjs v1 update. Stores that
    /// stream long payloads (whiteboard strokes) raise this above the doc
    /// module's default. The manager passes this into
    /// [`CollabDoc::apply_update_with_cap`](crate::collab::doc::CollabDoc::apply_update_with_cap).
    fn max_update_bytes(&self) -> usize {
        crate::collab::doc::MAX_DOC_BYTES
    }

    /// Maximum byte length of the full encoded doc state *after* applying an
    /// update. If exceeded, the manager rejects the update so the doc never
    /// grows past the cap (oversize docs can also block hydration on
    /// reconnect). Defaults to the same as `max_update_bytes`.
    fn max_doc_bytes(&self) -> usize {
        self.max_update_bytes()
    }
}

/// Build the right `state` ServerMessage for the resource kind.
pub(crate) fn state_message(r: &ResourceRef, snap: &Snapshot) -> ServerMessage {
    match r.kind {
        ResourceKind::Post => ServerMessage::CollabState {
            post_id: r.id.clone(),
            state_b64: snap.state_b64.clone(),
            state_vector_b64: snap.state_vector_b64.clone(),
        },
        ResourceKind::Whiteboard => ServerMessage::WhiteboardState {
            whiteboard_id: r.id.clone(),
            state_b64: snap.state_b64.clone(),
            state_vector_b64: snap.state_vector_b64.clone(),
        },
    }
}

pub(crate) fn update_message(
    r: &ResourceRef,
    update_b64: String,
    from_user: String,
) -> ServerMessage {
    match r.kind {
        ResourceKind::Post => ServerMessage::CollabUpdate {
            post_id: r.id.clone(),
            update_b64,
            from_user,
        },
        ResourceKind::Whiteboard => ServerMessage::WhiteboardUpdate {
            whiteboard_id: r.id.clone(),
            update_b64,
            from_user,
        },
    }
}

pub(crate) fn awareness_message(r: &ResourceRef, users: HashMap<String, Value>) -> ServerMessage {
    match r.kind {
        ResourceKind::Post => ServerMessage::AwarenessState {
            post_id: r.id.clone(),
            users,
        },
        ResourceKind::Whiteboard => ServerMessage::WhiteboardAwarenessState {
            whiteboard_id: r.id.clone(),
            users,
        },
    }
}

pub(crate) fn error_message(r: &ResourceRef, code: String, message: String) -> ServerMessage {
    match r.kind {
        ResourceKind::Post => ServerMessage::CollabError {
            post_id: r.id.clone(),
            code,
            message,
        },
        ResourceKind::Whiteboard => ServerMessage::WhiteboardError {
            whiteboard_id: r.id.clone(),
            code,
            message,
        },
    }
}

pub(crate) fn closed_message(r: &ResourceRef, reason: String) -> ServerMessage {
    match r.kind {
        ResourceKind::Post => ServerMessage::CollabClosed {
            post_id: r.id.clone(),
            reason,
        },
        ResourceKind::Whiteboard => ServerMessage::WhiteboardClosed {
            whiteboard_id: r.id.clone(),
            reason,
        },
    }
}
