use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::RecordId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: Option<RecordId>,
    pub name: String,
    pub channel_type: ChannelType,
    pub server: RecordId,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Text,
    Voice,
    Collab,
    /// Phase 3 — shared whiteboard channel. Each whiteboard channel has at
    /// most one persistent `whiteboard` record keyed by the channel ID.
    Whiteboard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub channel_type: ChannelType,
}
