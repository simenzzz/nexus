use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::repositories::Repos;

// ── Friend requests ──

#[derive(Debug, Deserialize)]
pub struct FriendRequestInput {
    pub user_id: String,
}

pub async fn send_friend_request(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Json(input): Json<FriendRequestInput>,
) -> Result<Json<Value>, AppError> {
    repos
        .social
        .send_friend_request(&claims.sub, &input.user_id)
        .await?;

    Ok(Json(json!({ "status": "sent" })))
}

#[derive(Debug, Deserialize)]
pub struct AcceptRequestInput {
    pub user_id: String,
}

pub async fn accept_friend_request(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Json(input): Json<AcceptRequestInput>,
) -> Result<Json<Value>, AppError> {
    repos
        .social
        .accept_friend_request(&input.user_id, &claims.sub)
        .await?;

    Ok(Json(json!({ "status": "accepted" })))
}

pub async fn remove_friend(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    repos.social.remove_friend(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "removed" })))
}

pub async fn list_friends(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let friends = repos.social.list_friends(&claims.sub).await?;

    Ok(Json(json!({ "friends": friends })))
}

pub async fn list_pending_incoming(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let pending = repos.social.list_pending_incoming(&claims.sub).await?;

    Ok(Json(json!({ "pending": pending })))
}

pub async fn get_mutual_friends(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mutual = repos
        .social
        .get_mutual_friends(&claims.sub, &user_id)
        .await?;

    Ok(Json(json!({ "mutual_friends": mutual })))
}

pub async fn get_friend_suggestions(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let suggestions = repos.social.get_friend_suggestions(&claims.sub, 20).await?;

    Ok(Json(json!({ "suggestions": suggestions })))
}

// ── Follow / Unfollow ──

pub async fn follow_user(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    repos.social.follow(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "following" })))
}

pub async fn unfollow_user(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    repos.social.unfollow(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "unfollowed" })))
}

// ── Block / Unblock ──

pub async fn block_user(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    repos.social.block_user(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "blocked" })))
}

pub async fn unblock_user(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    repos.social.unblock_user(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "unblocked" })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::Claims;
    use crate::models::user::{User, UserStatus};
    use crate::repositories::{
        channel::MockChannelRepo, message::MockMessageRepo, post::MockPostRepo,
        recommendations::MockRecommendationsRepo, server::MockServerRepo,
        social::MockSocialRepo, user::MockUserRepo, watch::MockWatchRepo,
        whiteboard::MockWhiteboardRepo,
    };
    use mockall::predicate::eq;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    fn claims(user_id: &str) -> Claims {
        Claims {
            sub: user_id.into(),
            token_type: "access".into(),
            iat: 0,
            exp: usize::MAX / 2,
        }
    }

    fn user(id: &str, username: &str) -> User {
        User {
            id: Some(surrealdb::RecordId::from(("user", id))),
            username: username.into(),
            display_name: username.into(),
            avatar_url: None,
            status: UserStatus::Online,
            created_at: None,
        }
    }

    fn repos_with_social(social: MockSocialRepo) -> Repos {
        Repos {
            users: Arc::new(MockUserRepo::new()),
            servers: Arc::new(MockServerRepo::new()),
            channels: Arc::new(MockChannelRepo::new()),
            messages: Arc::new(MockMessageRepo::new()),
            social: Arc::new(social),
            posts: Arc::new(MockPostRepo::new()),
            whiteboards: Arc::new(MockWhiteboardRepo::new()),
            watch: Arc::new(MockWatchRepo::new()),
            recommendations: Arc::new(MockRecommendationsRepo::new()),
        }
    }

    fn body_field(value: &Value, key: &str) -> String {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    #[tokio::test]
    async fn send_friend_request_forwards_caller_and_target() {
        let mut social = MockSocialRepo::new();
        social
            .expect_send_friend_request()
            .with(eq("user1"), eq("user2"))
            .times(1)
            .returning(|_, _| Ok(()));

        let response = send_friend_request(
            State(repos_with_social(social)),
            AuthUser(claims("user1")),
            Json(FriendRequestInput { user_id: "user2".into() }),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(body_field(&response.0, "status"), "sent");
    }

    #[tokio::test]
    async fn send_friend_request_propagates_repo_error() {
        let mut social = MockSocialRepo::new();
        social
            .expect_send_friend_request()
            .returning(|_, _| Err(AppError::BadRequest("Cannot send friend request to yourself".into())));

        let result = send_friend_request(
            State(repos_with_social(social)),
            AuthUser(claims("user1")),
            Json(FriendRequestInput { user_id: "user1".into() }),
        )
        .await;

        match result {
            Err(AppError::BadRequest(msg)) => assert!(msg.contains("yourself")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn accept_friend_request_swaps_direction() {
        // Caller (claims.sub) is the recipient of the request; sender comes from the body.
        let mut social = MockSocialRepo::new();
        social
            .expect_accept_friend_request()
            .with(eq("sender"), eq("recipient"))
            .times(1)
            .returning(|_, _| Ok(()));

        let response = accept_friend_request(
            State(repos_with_social(social)),
            AuthUser(claims("recipient")),
            Json(AcceptRequestInput { user_id: "sender".into() }),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(body_field(&response.0, "status"), "accepted");
    }

    #[tokio::test]
    async fn remove_friend_uses_path_user_id() {
        let mut social = MockSocialRepo::new();
        social
            .expect_remove_friend()
            .with(eq("me"), eq("ex-friend"))
            .times(1)
            .returning(|_, _| Ok(()));

        let response = remove_friend(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
            Path("ex-friend".into()),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(body_field(&response.0, "status"), "removed");
    }

    #[tokio::test]
    async fn list_friends_returns_users_under_friends_key() {
        let mut social = MockSocialRepo::new();
        social
            .expect_list_friends()
            .with(eq("me"))
            .returning(|_| Ok(vec![user("u2", "alice"), user("u3", "bob")]));

        let response = list_friends(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
        )
        .await
        .expect("handler should succeed");

        let friends = response.0.get("friends").and_then(Value::as_array).expect("friends array");
        assert_eq!(friends.len(), 2);
    }

    #[tokio::test]
    async fn list_pending_incoming_returns_requesters() {
        let mut social = MockSocialRepo::new();
        social
            .expect_list_pending_incoming()
            .with(eq("me"))
            .returning(|_| Ok(vec![user("u9", "carol")]));

        let response = list_pending_incoming(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
        )
        .await
        .expect("handler should succeed");

        let pending = response.0.get("pending").and_then(Value::as_array).expect("pending array");
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn get_mutual_friends_passes_both_user_ids() {
        let mut social = MockSocialRepo::new();
        social
            .expect_get_mutual_friends()
            .with(eq("me"), eq("them"))
            .returning(|_, _| Ok(vec![user("u4", "shared")]));

        let response = get_mutual_friends(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
            Path("them".into()),
        )
        .await
        .expect("handler should succeed");

        let mutual = response
            .0
            .get("mutual_friends")
            .and_then(Value::as_array)
            .expect("mutual_friends array");
        assert_eq!(mutual.len(), 1);
    }

    #[tokio::test]
    async fn get_friend_suggestions_uses_default_limit_of_20() {
        let mut social = MockSocialRepo::new();
        social
            .expect_get_friend_suggestions()
            .with(eq("me"), eq(20u32))
            .returning(|_, _| Ok(vec![]));

        let response = get_friend_suggestions(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
        )
        .await
        .expect("handler should succeed");

        let suggestions = response
            .0
            .get("suggestions")
            .and_then(Value::as_array)
            .expect("suggestions array");
        assert!(suggestions.is_empty());
    }

    #[tokio::test]
    async fn follow_user_delegates() {
        let mut social = MockSocialRepo::new();
        social
            .expect_follow()
            .with(eq("me"), eq("celeb"))
            .times(1)
            .returning(|_, _| Ok(()));

        let response = follow_user(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
            Path("celeb".into()),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(body_field(&response.0, "status"), "following");
    }

    #[tokio::test]
    async fn unfollow_user_delegates() {
        let mut social = MockSocialRepo::new();
        social
            .expect_unfollow()
            .with(eq("me"), eq("celeb"))
            .times(1)
            .returning(|_, _| Ok(()));

        let response = unfollow_user(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
            Path("celeb".into()),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(body_field(&response.0, "status"), "unfollowed");
    }

    #[tokio::test]
    async fn block_user_delegates() {
        let mut social = MockSocialRepo::new();
        social
            .expect_block_user()
            .with(eq("me"), eq("troll"))
            .times(1)
            .returning(|_, _| Ok(()));

        let response = block_user(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
            Path("troll".into()),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(body_field(&response.0, "status"), "blocked");
    }

    #[tokio::test]
    async fn unblock_user_delegates() {
        let mut social = MockSocialRepo::new();
        social
            .expect_unblock_user()
            .with(eq("me"), eq("troll"))
            .times(1)
            .returning(|_, _| Ok(()));

        let response = unblock_user(
            State(repos_with_social(social)),
            AuthUser(claims("me")),
            Path("troll".into()),
        )
        .await
        .expect("handler should succeed");

        assert_eq!(body_field(&response.0, "status"), "unblocked");
    }
}
