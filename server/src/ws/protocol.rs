use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Auth {
        ticket: String,
    },
    Subscribe {
        channel_id: String,
        level: SubscriptionLevel,
    },
    Unsubscribe {
        channel_id: String,
    },
    ChatMessage {
        channel_id: String,
        content: String,
        nonce: String,
    },
    Typing {
        channel_id: String,
    },
    Resume {
        last_seq: HashMap<String, u64>,
    },
    Heartbeat,
    // ── Phase 2: collaborative editing ──
    CollabSubscribe {
        post_id: String,
    },
    CollabUnsubscribe {
        post_id: String,
    },
    CollabUpdate {
        post_id: String,
        /// Base64-encoded Yjs update bytes.
        update_b64: String,
    },
    AwarenessUpdate {
        post_id: String,
        /// Opaque awareness state (cursor pos, selection, idle ts) — passed
        /// through unchanged to other subscribers.
        state: serde_json::Value,
    },
    // ── Phase 3: shared whiteboard (CRDT canvas) ──
    WhiteboardSubscribe {
        whiteboard_id: String,
    },
    WhiteboardUnsubscribe {
        whiteboard_id: String,
    },
    WhiteboardUpdate {
        whiteboard_id: String,
        update_b64: String,
    },
    WhiteboardAwarenessUpdate {
        whiteboard_id: String,
        state: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionLevel {
    Active,
    Badge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    AuthOk {
        user_id: String,
        heartbeat_interval: u64,
    },
    ChatMessage {
        seq: u64,
        channel_id: String,
        message_id: String,
        author: MessageAuthor,
        content: String,
        ts: u64,
    },
    MessageAck {
        nonce: String,
        message_id: String,
        seq: u64,
        ts: u64,
    },
    Typing {
        channel_id: String,
        user_id: String,
        username: String,
    },
    Presence {
        user_id: String,
        status: String,
    },
    Unread {
        channel_id: String,
        count: u64,
        last_message_preview: String,
    },
    Resync {
        channel_id: String,
    },
    HeartbeatAck,
    Error {
        message: String,
    },
    // ── Phase 2: collaborative editing ──
    CollabState {
        post_id: String,
        /// Base64-encoded full Yjs state (sent on subscribe so the client can
        /// hydrate its local Y.Doc without paying for replay history).
        state_b64: String,
        /// Base64-encoded state vector (so client can request a diff later).
        state_vector_b64: String,
    },
    CollabUpdate {
        post_id: String,
        update_b64: String,
        from_user: String,
    },
    AwarenessState {
        post_id: String,
        /// `user_id -> opaque state`
        users: HashMap<String, serde_json::Value>,
    },
    CollabError {
        post_id: String,
        code: String,
        message: String,
    },
    /// Sent when the server tears down a collab session — currently fires on
    /// publish so editors can flip to a read-only view.
    CollabClosed {
        post_id: String,
        reason: String,
    },
    // ── Phase 3: shared whiteboard ──
    WhiteboardState {
        whiteboard_id: String,
        state_b64: String,
        state_vector_b64: String,
    },
    WhiteboardUpdate {
        whiteboard_id: String,
        update_b64: String,
        from_user: String,
    },
    WhiteboardAwarenessState {
        whiteboard_id: String,
        users: HashMap<String, serde_json::Value>,
    },
    WhiteboardError {
        whiteboard_id: String,
        code: String,
        message: String,
    },
    /// Sent when the server tears down a whiteboard session (e.g., checkpoint
    /// restore). Clients should re-subscribe to fetch fresh state.
    WhiteboardClosed {
        whiteboard_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAuthor {
    pub id: String,
    pub username: String,
    pub avatar_url: Option<String>,
}

impl ServerMessage {
    pub fn to_json(&self) -> String {
        match serde_json::to_value(self) {
            Ok(mut value) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("v".to_string(), serde_json::json!(1));
                }
                serde_json::to_string(&value).unwrap_or_else(|e| {
                    tracing::error!(error = %e, "Failed to serialize ServerMessage");
                    r#"{"type":"error","message":"serialization failure","v":1}"#.to_string()
                })
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to convert ServerMessage to value");
                r#"{"type":"error","message":"serialization failure","v":1}"#.to_string()
            }
        }
    }
}
