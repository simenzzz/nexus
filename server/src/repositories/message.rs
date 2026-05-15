use async_trait::async_trait;
use serde::Serialize;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use crate::error::AppError;
use crate::models::message::Message;

#[derive(Debug, Serialize)]
pub struct CreateMessageDb {
    pub content: String,
    pub author: surrealdb::RecordId,
    pub channel: surrealdb::RecordId,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub trait MessageRepo: Send + Sync {
    async fn create(
        &self,
        content: String,
        author_id: &str,
        channel_id: &str,
    ) -> Result<Message, AppError>;
    async fn create_with_id(
        &self,
        id: &str,
        content: String,
        author_id: &str,
        channel_id: &str,
    ) -> Result<Message, AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Message>, AppError>;
    async fn list_for_channel(
        &self,
        channel_id: &str,
        before: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Message>, AppError>;
}

pub struct SurrealMessageRepo {
    db: Surreal<Client>,
}

impl SurrealMessageRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl MessageRepo for SurrealMessageRepo {
    async fn create(
        &self,
        content: String,
        author_id: &str,
        channel_id: &str,
    ) -> Result<Message, AppError> {
        let record: Option<Message> = self
            .db
            .create("message")
            .content(CreateMessageDb {
                content,
                author: surrealdb::RecordId::from(("user", author_id)),
                channel: surrealdb::RecordId::from(("channel", channel_id)),
                created_at: chrono::Utc::now(),
            })
            .await?;
        record.ok_or_else(|| AppError::Internal("Failed to create message".into()))
    }

    async fn create_with_id(
        &self,
        id: &str,
        content: String,
        author_id: &str,
        channel_id: &str,
    ) -> Result<Message, AppError> {
        let record: Option<Message> = self
            .db
            .create(("message", id))
            .content(CreateMessageDb {
                content,
                author: surrealdb::RecordId::from(("user", author_id)),
                channel: surrealdb::RecordId::from(("channel", channel_id)),
                created_at: chrono::Utc::now(),
            })
            .await?;
        record.ok_or_else(|| AppError::Internal("Failed to create message".into()))
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Message>, AppError> {
        let message: Option<Message> = self.db.select(("message", id)).await?;
        Ok(message)
    }

    async fn list_for_channel(
        &self,
        channel_id: &str,
        before: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Message>, AppError> {
        let query = match before {
            Some(_) => {
                "SELECT * FROM message WHERE channel = $channel \
                 AND created_at < (SELECT created_at FROM message WHERE id = $before LIMIT 1) \
                 ORDER BY created_at DESC LIMIT $limit"
            }
            None => {
                "SELECT * FROM message WHERE channel = $channel \
                 ORDER BY created_at DESC LIMIT $limit"
            }
        };

        let mut q = self
            .db
            .query(query)
            .bind(("channel", surrealdb::RecordId::from(("channel", channel_id))))
            .bind(("limit", limit));

        if let Some(before_id) = before {
            q = q.bind((
                "before",
                surrealdb::RecordId::from(("message", before_id)),
            ));
        }

        let mut result = q.await?;
        let messages: Vec<Message> = result.take(0)?;
        Ok(messages)
    }
}
