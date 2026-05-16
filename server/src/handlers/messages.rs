use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::repositories::Repos;

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub before: Option<String>,
    pub limit: Option<u32>,
}

pub async fn get_messages(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(channel_id): Path<String>,
    Query(params): Query<ListMessagesQuery>,
) -> Result<Json<Value>, AppError> {
    let channel = repos
        .channels
        .find_by_id(&channel_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;

    let server_key = channel.server.key().to_string();
    if !repos.servers.is_member(&server_key, &claims.sub).await? {
        return Err(AppError::Forbidden("Not a member of this server".into()));
    }

    let limit = params.limit.unwrap_or(50).min(100);
    let mut messages = repos
        .messages
        .list_for_channel(&channel_id, params.before.clone(), limit)
        .await?;

    // Repository returns DESC (newest first); reverse to ASC for display
    messages.reverse();

    Ok(Json(json!({ "messages": messages })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::Claims;
    use crate::models::channel::{Channel, ChannelType};
    use crate::models::message::Message;
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

    fn channel(id: &str, server_id: &str) -> Channel {
        Channel {
            id: Some(surrealdb::RecordId::from(("channel", id))),
            name: "general".into(),
            channel_type: ChannelType::Text,
            server: surrealdb::RecordId::from(("server", server_id)),
            created_at: None,
        }
    }

    fn message(id: &str, channel_id: &str, content: &str, ts: i64) -> Message {
        Message {
            id: Some(surrealdb::RecordId::from(("message", id))),
            content: content.into(),
            author: surrealdb::RecordId::from(("user", "u1")),
            channel: surrealdb::RecordId::from(("channel", channel_id)),
            created_at: chrono::DateTime::from_timestamp(ts, 0),
            edited_at: None,
        }
    }

    fn repos(
        channels: MockChannelRepo,
        servers: MockServerRepo,
        messages: MockMessageRepo,
    ) -> Repos {
        Repos {
            users: Arc::new(MockUserRepo::new()),
            servers: Arc::new(servers),
            channels: Arc::new(channels),
            messages: Arc::new(messages),
            social: Arc::new(MockSocialRepo::new()),
            posts: Arc::new(MockPostRepo::new()),
            whiteboards: Arc::new(MockWhiteboardRepo::new()),
            watch: Arc::new(MockWatchRepo::new()),
            recommendations: Arc::new(MockRecommendationsRepo::new()),
        }
    }

    #[tokio::test]
    async fn get_messages_returns_404_when_channel_missing() {
        let mut channels = MockChannelRepo::new();
        channels
            .expect_find_by_id()
            .with(eq("c1"))
            .returning(|_| Ok(None));

        let result = get_messages(
            State(repos(channels, MockServerRepo::new(), MockMessageRepo::new())),
            AuthUser(claims("u1")),
            Path("c1".into()),
            Query(ListMessagesQuery { before: None, limit: None }),
        )
        .await;

        match result {
            Err(AppError::NotFound(msg)) => assert!(msg.contains("Channel")),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_messages_returns_403_when_not_server_member() {
        let mut channels = MockChannelRepo::new();
        channels
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1"))));

        let mut servers = MockServerRepo::new();
        servers
            .expect_is_member()
            .with(eq("s1"), eq("u1"))
            .returning(|_, _| Ok(false));

        let result = get_messages(
            State(repos(channels, servers, MockMessageRepo::new())),
            AuthUser(claims("u1")),
            Path("c1".into()),
            Query(ListMessagesQuery { before: None, limit: None }),
        )
        .await;

        match result {
            Err(AppError::Forbidden(_)) => {}
            other => panic!("expected Forbidden, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_messages_default_limit_is_50() {
        let mut channels = MockChannelRepo::new();
        channels
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1"))));

        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));

        let mut messages = MockMessageRepo::new();
        messages
            .expect_list_for_channel()
            .with(eq("c1"), eq(None::<String>), eq(50u32))
            .returning(|_, _, _| Ok(vec![]));

        let response = get_messages(
            State(repos(channels, servers, messages)),
            AuthUser(claims("u1")),
            Path("c1".into()),
            Query(ListMessagesQuery { before: None, limit: None }),
        )
        .await
        .expect("handler should succeed");

        let arr = response.0.get("messages").and_then(Value::as_array).expect("messages array");
        assert_eq!(arr.len(), 0);
    }

    #[tokio::test]
    async fn get_messages_caps_limit_at_100() {
        let mut channels = MockChannelRepo::new();
        channels
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1"))));

        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));

        let mut messages = MockMessageRepo::new();
        // Request 500, expect 100 to be passed to repo.
        messages
            .expect_list_for_channel()
            .with(eq("c1"), eq(None::<String>), eq(100u32))
            .returning(|_, _, _| Ok(vec![]));

        let _ = get_messages(
            State(repos(channels, servers, messages)),
            AuthUser(claims("u1")),
            Path("c1".into()),
            Query(ListMessagesQuery { before: None, limit: Some(500) }),
        )
        .await
        .expect("handler should succeed");
    }

    #[tokio::test]
    async fn get_messages_reverses_repo_order_for_display() {
        let mut channels = MockChannelRepo::new();
        channels
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1"))));

        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));

        let mut messages = MockMessageRepo::new();
        messages.expect_list_for_channel().returning(|_, _, _| {
            // Repo returns DESC (newest first)
            Ok(vec![
                message("m3", "c1", "third", 300),
                message("m2", "c1", "second", 200),
                message("m1", "c1", "first", 100),
            ])
        });

        let response = get_messages(
            State(repos(channels, servers, messages)),
            AuthUser(claims("u1")),
            Path("c1".into()),
            Query(ListMessagesQuery { before: None, limit: None }),
        )
        .await
        .expect("handler should succeed");

        let arr = response.0.get("messages").and_then(Value::as_array).expect("messages array");
        let contents: Vec<&str> = arr
            .iter()
            .map(|m| m.get("content").and_then(Value::as_str).unwrap())
            .collect();
        // Handler should reverse to ASC (oldest first) for display.
        assert_eq!(contents, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn get_messages_forwards_before_cursor() {
        let mut channels = MockChannelRepo::new();
        channels
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1"))));

        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));

        let mut messages = MockMessageRepo::new();
        messages
            .expect_list_for_channel()
            .with(eq("c1"), eq(Some("msg42".to_string())), eq(50u32))
            .returning(|_, _, _| Ok(vec![]));

        let _ = get_messages(
            State(repos(channels, servers, messages)),
            AuthUser(claims("u1")),
            Path("c1".into()),
            Query(ListMessagesQuery {
                before: Some("msg42".into()),
                limit: None,
            }),
        )
        .await
        .expect("handler should succeed");
    }
}
