use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::RecordId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub id: Option<RecordId>,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub owner: RecordId,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateServer {
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
}
