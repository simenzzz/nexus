use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::Serialize;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::error::AppError;
use crate::models::channel::{Channel, ChannelType, CreateChannel};

#[derive(Debug, Serialize)]
pub struct CreateChannelDb {
    pub name: String,
    pub channel_type: ChannelType,
    pub server: surrealdb::RecordId,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait ChannelRepo: Send + Sync {
    async fn create(&self, input: CreateChannel, server_id: &str) -> Result<Channel, AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Channel>, AppError>;
    async fn list_for_server(&self, server_id: &str) -> Result<Vec<Channel>, AppError>;
}

pub struct SurrealChannelRepo {
    db: Surreal<Client>,
}

impl SurrealChannelRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ChannelRepo for SurrealChannelRepo {
    async fn create(&self, input: CreateChannel, server_id: &str) -> Result<Channel, AppError> {
        let record: Option<Channel> = self
            .db
            .create("channel")
            .content(CreateChannelDb {
                name: input.name,
                channel_type: input.channel_type,
                server: surrealdb::RecordId::from(("server", server_id)),
                created_at: chrono::Utc::now(),
            })
            .await?;
        record.ok_or_else(|| AppError::Internal("Failed to create channel".into()))
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Channel>, AppError> {
        let channel: Option<Channel> = self.db.select(("channel", id)).await?;
        Ok(channel)
    }

    async fn list_for_server(&self, server_id: &str) -> Result<Vec<Channel>, AppError> {
        let mut result = self
            .db
            .query("SELECT * FROM channel WHERE server = $server_id")
            .bind((
                "server_id",
                surrealdb::RecordId::from(("server", server_id)),
            ))
            .await?;
        let channels: Vec<Channel> = result.take(0)?;
        Ok(channels)
    }
}
