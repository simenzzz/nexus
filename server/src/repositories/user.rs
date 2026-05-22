use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::error::AppError;
use crate::models::user::{CreateUser, User, UserStatus};

/// Internal struct for DB lookups that include the password hash.
/// Never serialized to clients.
#[derive(Debug, Deserialize)]
pub struct UserWithPassword {
    pub id: Option<surrealdb::RecordId>,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub status: UserStatus,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub password_hash: String,
}

impl From<UserWithPassword> for User {
    fn from(u: UserWithPassword) -> Self {
        Self {
            id: u.id,
            username: u.username,
            display_name: u.display_name,
            avatar_url: u.avatar_url,
            status: u.status,
            created_at: u.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CreateUserDb {
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub status: UserStatus,
    pub password_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn create(&self, input: CreateUser, password_hash: String) -> Result<User, AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<User>, AppError>;
    async fn find_by_username(&self, username: &str) -> Result<Option<UserWithPassword>, AppError>;
}

pub struct SurrealUserRepo {
    db: Surreal<Client>,
}

impl SurrealUserRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl UserRepo for SurrealUserRepo {
    async fn create(&self, input: CreateUser, password_hash: String) -> Result<User, AppError> {
        let record: Option<User> = self
            .db
            .create("user")
            .content(CreateUserDb {
                username: input.username,
                display_name: input.display_name,
                avatar_url: None,
                status: UserStatus::Online,
                password_hash,
                created_at: chrono::Utc::now(),
            })
            .await?;
        record.ok_or_else(|| AppError::Internal("Failed to create user".into()))
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<User>, AppError> {
        let user: Option<User> = self.db.select(("user", id)).await?;
        Ok(user)
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<UserWithPassword>, AppError> {
        let mut result = self
            .db
            .query("SELECT * FROM user WHERE username = $username LIMIT 1")
            .bind(("username", username.to_string()))
            .await?;
        let user: Option<UserWithPassword> = result.take(0)?;
        Ok(user)
    }
}
