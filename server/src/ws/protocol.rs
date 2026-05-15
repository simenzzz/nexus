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
