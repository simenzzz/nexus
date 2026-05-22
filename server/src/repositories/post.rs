use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::error::AppError;
use crate::models::post::Post;

#[derive(Debug, Serialize)]
struct CreatePostDb {
    author: surrealdb::RecordId,
    title: String,
    state_b64: String,
    state_vector_b64: String,
    published: bool,
    published_content: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct CountResult {
    count: u64,
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait PostRepo: Send + Sync {
    async fn create_draft(&self, author_id: &str, title: String) -> Result<Post, AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Post>, AppError>;
    async fn save_snapshot(
        &self,
        id: &str,
        state_b64: String,
        state_vector_b64: String,
    ) -> Result<(), AppError>;
    async fn publish(&self, id: &str, content: String) -> Result<Post, AppError>;
    /// Most-recent published posts, newest first.
    async fn list_published(&self, limit: u32) -> Result<Vec<Post>, AppError>;
    /// Returns true if there is an `invited_to` edge from `user_id` to the post.
    /// The author is not included — callers must `OR` against the author check.
    async fn is_invited(&self, post_id: &str, user_id: &str) -> Result<bool, AppError>;
    /// Idempotently create an `invited_to` edge. Existence/friendship checks
    /// live in the handler (cross-repo).
    async fn add_invite(&self, post_id: &str, invitee_id: &str) -> Result<(), AppError>;
}

pub struct SurrealPostRepo {
    db: Surreal<Client>,
}

impl SurrealPostRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl PostRepo for SurrealPostRepo {
    async fn create_draft(&self, author_id: &str, title: String) -> Result<Post, AppError> {
        let now = chrono::Utc::now();
        let record: Option<Post> = self
            .db
            .create("post")
            .content(CreatePostDb {
                author: surrealdb::RecordId::from(("user", author_id)),
                title,
                state_b64: String::new(),
                state_vector_b64: String::new(),
                published: false,
                published_content: None,
                created_at: now,
                updated_at: now,
            })
            .await?;
        record.ok_or_else(|| AppError::Internal("Failed to create post draft".into()))
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Post>, AppError> {
        let post: Option<Post> = self.db.select(("post", id)).await?;
        Ok(post)
    }

    async fn save_snapshot(
        &self,
        id: &str,
        state_b64: String,
        state_vector_b64: String,
    ) -> Result<(), AppError> {
        self.db
            .query(
                "UPDATE $id SET state_b64 = $state, state_vector_b64 = $sv, updated_at = time::now()",
            )
            .bind(("id", surrealdb::RecordId::from(("post", id))))
            .bind(("state", state_b64))
            .bind(("sv", state_vector_b64))
            .await?;
        Ok(())
    }

    async fn publish(&self, id: &str, content: String) -> Result<Post, AppError> {
        let mut result = self
            .db
            .query(
                "UPDATE $id SET published = true, published_content = $content, \
                 updated_at = time::now() RETURN AFTER",
            )
            .bind(("id", surrealdb::RecordId::from(("post", id))))
            .bind(("content", content))
            .await?;
        let updated: Vec<Post> = result.take(0)?;
        updated
            .into_iter()
            .next()
            .ok_or_else(|| AppError::NotFound("Post not found".into()))
    }

    async fn list_published(&self, limit: u32) -> Result<Vec<Post>, AppError> {
        let mut result = self
            .db
            .query(
                "SELECT * FROM post WHERE published = true \
                 ORDER BY updated_at DESC LIMIT $limit",
            )
            .bind(("limit", limit))
            .await?;
        let posts: Vec<Post> = result.take(0)?;
        Ok(posts)
    }

    async fn is_invited(&self, post_id: &str, user_id: &str) -> Result<bool, AppError> {
        let mut result = self
            .db
            .query(
                "SELECT count() AS count FROM invited_to \
                 WHERE in = $user AND out = $post GROUP BY count",
            )
            .bind(("user", surrealdb::RecordId::from(("user", user_id))))
            .bind(("post", surrealdb::RecordId::from(("post", post_id))))
            .await?;
        let counts: Vec<CountResult> = result.take(0)?;
        Ok(counts.first().map(|c| c.count > 0).unwrap_or(false))
    }

    async fn add_invite(&self, post_id: &str, invitee_id: &str) -> Result<(), AppError> {
        // Idempotent: skip if the edge already exists.
        if self.is_invited(post_id, invitee_id).await? {
            return Ok(());
        }
        self.db
            .query("RELATE $user -> invited_to -> $post SET created_at = time::now()")
            .bind(("user", surrealdb::RecordId::from(("user", invitee_id))))
            .bind(("post", surrealdb::RecordId::from(("post", post_id))))
            .await?;
        Ok(())
    }
}
