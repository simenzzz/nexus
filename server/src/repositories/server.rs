use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::Serialize;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::error::AppError;
use crate::models::server::{CreateServer, Server};

#[derive(Debug, Serialize)]
pub struct CreateServerDb {
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub owner: surrealdb::RecordId,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait ServerRepo: Send + Sync {
    async fn create(&self, input: CreateServer, owner_id: &str) -> Result<Server, AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Server>, AppError>;
    async fn list_for_user(&self, user_id: &str) -> Result<Vec<Server>, AppError>;
    async fn add_member(&self, server_id: &str, user_id: &str) -> Result<(), AppError>;
    async fn is_member(&self, server_id: &str, user_id: &str) -> Result<bool, AppError>;
    /// Distinct user ids who are members of any server the given user is in
    /// (excluding the user themselves). Used by the presence layer to scope
    /// online/offline broadcasts to users with an actual graph relationship.
    async fn list_co_member_ids(&self, user_id: &str) -> Result<Vec<String>, AppError>;
}

pub struct SurrealServerRepo {
    db: Surreal<Client>,
}

impl SurrealServerRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ServerRepo for SurrealServerRepo {
    async fn create(&self, input: CreateServer, owner_id: &str) -> Result<Server, AppError> {
        let record: Option<Server> = self
            .db
            .create("server")
            .content(CreateServerDb {
                name: input.name,
                description: input.description,
                icon_url: input.icon_url,
                owner: surrealdb::RecordId::from(("user", owner_id)),
                created_at: chrono::Utc::now(),
            })
            .await?;
        record.ok_or_else(|| AppError::Internal("Failed to create server".into()))
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Server>, AppError> {
        let server: Option<Server> = self.db.select(("server", id)).await?;
        Ok(server)
    }

    async fn list_for_user(&self, user_id: &str) -> Result<Vec<Server>, AppError> {
        let mut result = self
            .db
            .query(
                "SELECT * FROM server WHERE owner = $user_id \
                 OR id IN (SELECT VALUE out FROM member_of WHERE in = $user_id)",
            )
            .bind(("user_id", surrealdb::RecordId::from(("user", user_id))))
            .await?;
        let servers: Vec<Server> = result.take(0)?;
        Ok(servers)
    }

    async fn add_member(&self, server_id: &str, user_id: &str) -> Result<(), AppError> {
        // Check if already a member to prevent duplicate edges
        if self.is_member(server_id, user_id).await? {
            return Ok(());
        }
        self.db
            .query("RELATE $user -> member_of -> $server SET joined_at = time::now()")
            .bind(("user", surrealdb::RecordId::from(("user", user_id))))
            .bind(("server", surrealdb::RecordId::from(("server", server_id))))
            .await?;
        Ok(())
    }

    async fn list_co_member_ids(&self, user_id: &str) -> Result<Vec<String>, AppError> {
        // Two-hop: user → member_of → server → member_of → other_user.
        // Excludes self AND any user the caller has blocked (or who has
        // blocked the caller) — block semantics promise mutual presence
        // invisibility, so the audience set must respect both directions.
        // GROUP BY in deduplicates users in multiple shared servers.
        let mut result = self
            .db
            .query(
                "SELECT VALUE meta::id(in) FROM member_of \
                 WHERE out IN (SELECT VALUE out FROM member_of WHERE in = $user) \
                 AND in != $user \
                 AND in NOT IN (SELECT VALUE out FROM blocked WHERE in = $user) \
                 AND in NOT IN (SELECT VALUE in FROM blocked WHERE out = $user) \
                 GROUP BY in",
            )
            .bind(("user", surrealdb::RecordId::from(("user", user_id))))
            .await?;
        let ids: Vec<String> = result.take(0)?;
        Ok(ids)
    }

    async fn is_member(&self, server_id: &str, user_id: &str) -> Result<bool, AppError> {
        let mut result = self
            .db
            .query(
                "SELECT count() AS count FROM member_of \
                 WHERE in = $user AND out = $server GROUP BY count",
            )
            .bind(("user", surrealdb::RecordId::from(("user", user_id))))
            .bind(("server", surrealdb::RecordId::from(("server", server_id))))
            .await?;

        #[derive(serde::Deserialize)]
        struct CountResult {
            count: u64,
        }

        let counts: Vec<CountResult> = result.take(0)?;
        Ok(counts.first().map(|c| c.count > 0).unwrap_or(false))
    }
}
