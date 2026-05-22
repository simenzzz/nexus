use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::Deserialize;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::error::AppError;
use crate::models::user::User;

#[cfg_attr(test, automock)]
#[async_trait]
pub trait SocialRepo: Send + Sync {
    async fn send_friend_request(&self, from: &str, to: &str) -> Result<(), AppError>;
    async fn accept_friend_request(&self, from: &str, to: &str) -> Result<(), AppError>;
    async fn remove_friend(&self, user: &str, friend: &str) -> Result<(), AppError>;
    async fn list_friends(&self, user: &str) -> Result<Vec<User>, AppError>;
    async fn list_pending_incoming(&self, user: &str) -> Result<Vec<User>, AppError>;
    async fn are_friends(&self, a: &str, b: &str) -> Result<bool, AppError>;
    async fn get_mutual_friends(&self, a: &str, b: &str) -> Result<Vec<User>, AppError>;
    async fn get_friend_suggestions(&self, user: &str, limit: u32) -> Result<Vec<User>, AppError>;
    async fn follow(&self, follower: &str, followed: &str) -> Result<(), AppError>;
    async fn unfollow(&self, follower: &str, followed: &str) -> Result<(), AppError>;
    async fn block_user(&self, blocker: &str, blocked: &str) -> Result<(), AppError>;
    async fn unblock_user(&self, blocker: &str, blocked: &str) -> Result<(), AppError>;
    async fn is_blocked(&self, blocker: &str, blocked: &str) -> Result<bool, AppError>;
    async fn get_friend_ids(&self, user: &str) -> Result<Vec<String>, AppError>;
}

pub struct SurrealSocialRepo {
    db: Surreal<Client>,
}

impl SurrealSocialRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[derive(Debug, Deserialize)]
struct CountResult {
    count: u64,
}

#[async_trait]
impl SocialRepo for SurrealSocialRepo {
    async fn send_friend_request(&self, from: &str, to: &str) -> Result<(), AppError> {
        // Prevent self-request
        if from == to {
            return Err(AppError::BadRequest(
                "Cannot send friend request to yourself".into(),
            ));
        }

        // Verify target user exists
        let target: Option<User> = self.db.select(("user", to)).await?;
        if target.is_none() {
            return Err(AppError::NotFound("User not found".into()));
        }

        // Check not already friends
        if self.are_friends(from, to).await? {
            return Err(AppError::BadRequest(
                "Already friends with this user".into(),
            ));
        }

        // Check blocked in either direction
        if self.is_blocked(to, from).await? || self.is_blocked(from, to).await? {
            return Err(AppError::BadRequest(
                "Cannot send friend request to this user".into(),
            ));
        }

        // Check for existing pending request (prevent duplicates)
        let mut existing = self
            .db
            .query(
                "SELECT count() AS count FROM friends_with \
                 WHERE in = $from AND out = $to GROUP BY count",
            )
            .bind(("from", surrealdb::RecordId::from(("user", from))))
            .bind(("to", surrealdb::RecordId::from(("user", to))))
            .await?;
        let counts: Vec<CountResult> = existing.take(0)?;
        if counts.first().map(|c| c.count > 0).unwrap_or(false) {
            return Err(AppError::BadRequest("Friend request already sent".into()));
        }

        self.db
            .query(
                "RELATE $from -> friends_with -> $to SET status = 'pending', created_at = time::now()",
            )
            .bind(("from", surrealdb::RecordId::from(("user", from))))
            .bind(("to", surrealdb::RecordId::from(("user", to))))
            .await?;
        Ok(())
    }

    async fn accept_friend_request(&self, from: &str, to: &str) -> Result<(), AppError> {
        // Update pending edge from->to to 'accepted' and verify it existed
        let mut result = self
            .db
            .query(
                "UPDATE friends_with SET status = 'accepted' \
                 WHERE in = $from AND out = $to AND status = 'pending' \
                 RETURN AFTER",
            )
            .bind(("from", surrealdb::RecordId::from(("user", from))))
            .bind(("to", surrealdb::RecordId::from(("user", to))))
            .await?;

        let updated: Vec<serde_json::Value> = result.take(0)?;
        if updated.is_empty() {
            return Err(AppError::NotFound("No pending friend request found".into()));
        }

        // Create reciprocal edge to->from
        self.db
            .query(
                "RELATE $to -> friends_with -> $from SET status = 'accepted', created_at = time::now()",
            )
            .bind(("to", surrealdb::RecordId::from(("user", to))))
            .bind(("from", surrealdb::RecordId::from(("user", from))))
            .await?;
        Ok(())
    }

    async fn remove_friend(&self, user: &str, friend: &str) -> Result<(), AppError> {
        // Delete edges in both directions
        self.db
            .query(
                "DELETE friends_with WHERE (in = $user AND out = $friend) OR (in = $friend AND out = $user)",
            )
            .bind(("user", surrealdb::RecordId::from(("user", user))))
            .bind(("friend", surrealdb::RecordId::from(("user", friend))))
            .await?;
        Ok(())
    }

    async fn list_friends(&self, user: &str) -> Result<Vec<User>, AppError> {
        let mut result = self
            .db
            .query("SELECT VALUE out.* FROM friends_with WHERE in = $user AND status = 'accepted'")
            .bind(("user", surrealdb::RecordId::from(("user", user))))
            .await?;
        let friends: Vec<User> = result.take(0)?;
        Ok(friends)
    }

    async fn list_pending_incoming(&self, user: &str) -> Result<Vec<User>, AppError> {
        let mut result = self
            .db
            .query("SELECT VALUE in.* FROM friends_with WHERE out = $user AND status = 'pending'")
            .bind(("user", surrealdb::RecordId::from(("user", user))))
            .await?;
        let pending: Vec<User> = result.take(0)?;
        Ok(pending)
    }

    async fn are_friends(&self, a: &str, b: &str) -> Result<bool, AppError> {
        let mut result = self
            .db
            .query(
                "SELECT count() AS count FROM friends_with WHERE in = $a AND out = $b AND status = 'accepted' GROUP BY count",
            )
            .bind(("a", surrealdb::RecordId::from(("user", a))))
            .bind(("b", surrealdb::RecordId::from(("user", b))))
            .await?;
        let counts: Vec<CountResult> = result.take(0)?;
        Ok(counts.first().map(|c| c.count > 0).unwrap_or(false))
    }

    async fn get_mutual_friends(&self, a: &str, b: &str) -> Result<Vec<User>, AppError> {
        // Find users who are accepted friends of both a and b
        let mut result = self
            .db
            .query(
                "SELECT * FROM user WHERE id IN (SELECT VALUE out FROM friends_with WHERE in = $a AND status = 'accepted') \
                 AND id IN (SELECT VALUE out FROM friends_with WHERE in = $b AND status = 'accepted')",
            )
            .bind(("a", surrealdb::RecordId::from(("user", a))))
            .bind(("b", surrealdb::RecordId::from(("user", b))))
            .await?;
        let mutual: Vec<User> = result.take(0)?;
        Ok(mutual)
    }

    async fn get_friend_suggestions(&self, user: &str, limit: u32) -> Result<Vec<User>, AppError> {
        // 2-hop traversal: friends-of-friends, excluding existing friends and self
        // Step 1: Get IDs of my friends
        let my_friend_ids = self.get_friend_ids(user).await?;
        if my_friend_ids.is_empty() {
            return Ok(vec![]);
        }

        let my_friend_record_ids: Vec<surrealdb::RecordId> = my_friend_ids
            .iter()
            .map(|id| surrealdb::RecordId::from(("user", id.as_str())))
            .collect();

        // Step 2: Find friends-of-friends, excluding self and existing friends
        let mut result = self
            .db
            .query(
                "SELECT VALUE out FROM friends_with \
                 WHERE in IN $friends AND status = 'accepted' \
                 AND out != $user \
                 AND out NOT IN (SELECT VALUE out FROM friends_with WHERE in = $user) \
                 GROUP BY out LIMIT $limit",
            )
            .bind(("friends", my_friend_record_ids))
            .bind(("user", surrealdb::RecordId::from(("user", user))))
            .bind(("limit", limit))
            .await?;

        let suggestion_ids: Vec<surrealdb::RecordId> = result.take(0)?;

        let mut users = Vec::new();
        for rid in suggestion_ids {
            let key = rid.key().to_string();
            if let Ok(Some(u)) = self.db.select::<Option<User>>(("user", key.as_str())).await {
                users.push(u);
            }
        }

        Ok(users)
    }

    async fn follow(&self, follower: &str, followed: &str) -> Result<(), AppError> {
        if follower == followed {
            return Err(AppError::BadRequest("Cannot follow yourself".into()));
        }

        // Prevent duplicate follow edges
        let mut existing = self
            .db
            .query(
                "SELECT count() AS count FROM follows \
                 WHERE in = $follower AND out = $followed GROUP BY count",
            )
            .bind(("follower", surrealdb::RecordId::from(("user", follower))))
            .bind(("followed", surrealdb::RecordId::from(("user", followed))))
            .await?;
        let counts: Vec<CountResult> = existing.take(0)?;
        if counts.first().map(|c| c.count > 0).unwrap_or(false) {
            return Ok(());
        }

        self.db
            .query("RELATE $follower -> follows -> $followed SET created_at = time::now()")
            .bind(("follower", surrealdb::RecordId::from(("user", follower))))
            .bind(("followed", surrealdb::RecordId::from(("user", followed))))
            .await?;
        Ok(())
    }

    async fn unfollow(&self, follower: &str, followed: &str) -> Result<(), AppError> {
        self.db
            .query("DELETE follows WHERE in = $follower AND out = $followed")
            .bind(("follower", surrealdb::RecordId::from(("user", follower))))
            .bind(("followed", surrealdb::RecordId::from(("user", followed))))
            .await?;
        Ok(())
    }

    async fn block_user(&self, blocker: &str, blocked: &str) -> Result<(), AppError> {
        if blocker == blocked {
            return Err(AppError::BadRequest("Cannot block yourself".into()));
        }

        // Check if already blocked (prevent duplicate edges)
        if self.is_blocked(blocker, blocked).await? {
            return Ok(());
        }

        // Remove any friendship in both directions (propagate errors)
        self.remove_friend(blocker, blocked).await?;

        self.db
            .query("RELATE $blocker -> blocked -> $blocked SET created_at = time::now()")
            .bind(("blocker", surrealdb::RecordId::from(("user", blocker))))
            .bind(("blocked", surrealdb::RecordId::from(("user", blocked))))
            .await?;
        Ok(())
    }

    async fn unblock_user(&self, blocker: &str, blocked: &str) -> Result<(), AppError> {
        self.db
            .query("DELETE blocked WHERE in = $blocker AND out = $blocked")
            .bind(("blocker", surrealdb::RecordId::from(("user", blocker))))
            .bind(("blocked", surrealdb::RecordId::from(("user", blocked))))
            .await?;
        Ok(())
    }

    async fn is_blocked(&self, blocker: &str, blocked: &str) -> Result<bool, AppError> {
        let mut result = self
            .db
            .query(
                "SELECT count() AS count FROM blocked WHERE in = $blocker AND out = $blocked GROUP BY count",
            )
            .bind(("blocker", surrealdb::RecordId::from(("user", blocker))))
            .bind(("blocked", surrealdb::RecordId::from(("user", blocked))))
            .await?;
        let counts: Vec<CountResult> = result.take(0)?;
        Ok(counts.first().map(|c| c.count > 0).unwrap_or(false))
    }

    async fn get_friend_ids(&self, user: &str) -> Result<Vec<String>, AppError> {
        let mut result = self
            .db
            .query(
                "SELECT VALUE meta::id(out) FROM friends_with WHERE in = $user AND status = 'accepted'",
            )
            .bind(("user", surrealdb::RecordId::from(("user", user))))
            .await?;
        let ids: Vec<String> = result.take(0)?;
        Ok(ids)
    }
}
