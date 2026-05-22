use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::collab::doc::CollabDoc;
use crate::collab::resource::ResourceRef;
use crate::collab::CollabManager;
use crate::error::AppError;
use crate::repositories::Repos;

#[derive(Debug, Deserialize)]
pub struct CreateDraftInput {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct InviteInput {
    pub user_id: String,
}

pub async fn create_draft(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Json(input): Json<CreateDraftInput>,
) -> Result<Json<Value>, AppError> {
    let title = input.title.trim();
    if title.is_empty() || title.len() > 200 {
        return Err(AppError::BadRequest(
            "Title must be 1-200 characters".into(),
        ));
    }

    let post = repos
        .posts
        .create_draft(&claims.sub, title.to_string())
        .await?;
    Ok(Json(json!({ "post": post })))
}

pub async fn get_post(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(post_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let post = repos
        .posts
        .find_by_id(&post_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;

    // Published posts are public reads (feed surfaces them anyway). Drafts
    // are only visible to the author and explicitly invited collaborators.
    if !post.published {
        let author_key = post.author.key().to_string();
        let allowed =
            author_key == claims.sub || repos.posts.is_invited(&post_id, &claims.sub).await?;
        if !allowed {
            return Err(AppError::Forbidden(
                "Not authorized to view this draft".into(),
            ));
        }
    }
    Ok(Json(json!({ "post": post })))
}

pub async fn invite_collaborator(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(post_id): Path<String>,
    Json(input): Json<InviteInput>,
) -> Result<Json<Value>, AppError> {
    let post = repos
        .posts
        .find_by_id(&post_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;

    let author_key = post.author.key().to_string();
    if author_key != claims.sub {
        return Err(AppError::Forbidden(
            "Only the author can invite collaborators".into(),
        ));
    }
    if post.published {
        return Err(AppError::BadRequest(
            "Cannot invite collaborators to a published post".into(),
        ));
    }
    let invitee = input.user_id.trim();
    if invitee.is_empty() {
        return Err(AppError::BadRequest("user_id is required".into()));
    }
    if invitee == claims.sub {
        return Err(AppError::BadRequest(
            "Author is already a collaborator".into(),
        ));
    }
    // Roadmap §2.2 — eligibility is determined by the social graph. Today
    // that's accepted friends; server-membership eligibility will be added
    // when posts gain a server scope.
    let are_friends = repos.social.are_friends(&claims.sub, invitee).await?;
    if !are_friends {
        return Err(AppError::Forbidden(
            "Can only invite accepted friends as collaborators".into(),
        ));
    }
    repos.posts.add_invite(&post_id, invitee).await?;
    Ok(Json(
        json!({ "ok": true, "post_id": post_id, "user_id": invitee }),
    ))
}

pub async fn publish_post(
    State(repos): State<Repos>,
    State(collab): State<CollabManager>,
    AuthUser(claims): AuthUser,
    Path(post_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let post = repos
        .posts
        .find_by_id(&post_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Post not found".into()))?;

    let author_key = post.author.key().to_string();
    if author_key != claims.sub {
        return Err(AppError::Forbidden("Only the author can publish".into()));
    }
    if post.published {
        return Err(AppError::BadRequest("Post already published".into()));
    }

    // Freeze the Y.Doc by extracting its current plain-text content and
    // persisting it as immutable `published_content`. The CRDT state stays
    // around in case we want to support post-publish edits later.
    let doc = CollabDoc::from_snapshot(&post.state_b64)?;
    let content = doc.text();

    let updated = repos.posts.publish(&post_id, content).await?;

    // Notify any active editors that the room is closing, then drop the
    // cached session so further `CollabUpdate` messages are rejected
    // (subscribe will refuse because the post is now `published == true`).
    let _ = collab
        .close(&ResourceRef::post(post_id.clone()), "published")
        .await;

    Ok(Json(json!({ "post": updated })))
}

pub async fn list_published(
    State(repos): State<Repos>,
    AuthUser(_claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let posts = repos.posts.list_published(50).await?;
    Ok(Json(json!({ "posts": posts })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::Claims;
    use crate::models::post::Post;
    use crate::repositories::{
        channel::MockChannelRepo, message::MockMessageRepo, post::MockPostRepo,
        recommendations::MockRecommendationsRepo, server::MockServerRepo, social::MockSocialRepo,
        user::MockUserRepo, watch::MockWatchRepo, whiteboard::MockWhiteboardRepo,
    };
    use mockall::predicate::eq;
    use std::sync::Arc;

    fn claims(user_id: &str) -> Claims {
        Claims {
            sub: user_id.into(),
            token_type: "access".into(),
            iat: 0,
            exp: usize::MAX / 2,
        }
    }

    fn post(id: &str, author: &str, published: bool) -> Post {
        Post {
            id: Some(surrealdb::RecordId::from(("post", id))),
            author: surrealdb::RecordId::from(("user", author)),
            title: "draft".into(),
            state_b64: String::new(),
            state_vector_b64: String::new(),
            published,
            published_content: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn repos(posts: MockPostRepo) -> Repos {
        Repos {
            users: Arc::new(MockUserRepo::new()),
            servers: Arc::new(MockServerRepo::new()),
            channels: Arc::new(MockChannelRepo::new()),
            messages: Arc::new(MockMessageRepo::new()),
            social: Arc::new(MockSocialRepo::new()),
            posts: Arc::new(posts),
            whiteboards: Arc::new(MockWhiteboardRepo::new()),
            watch: Arc::new(MockWatchRepo::new()),
            recommendations: Arc::new(MockRecommendationsRepo::new()),
        }
    }

    /// Build a CollabManager backed by a no-op MockPostRepo. Sufficient for
    /// handler tests where close_post / authz never fire (no live sessions).
    fn collab() -> CollabManager {
        CollabManager::new(Arc::new(MockPostRepo::new()))
    }

    #[tokio::test]
    async fn create_draft_rejects_empty_title() {
        let result = create_draft(
            State(repos(MockPostRepo::new())),
            AuthUser(claims("u1")),
            Json(CreateDraftInput {
                title: "   ".into(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn create_draft_rejects_oversize_title() {
        let result = create_draft(
            State(repos(MockPostRepo::new())),
            AuthUser(claims("u1")),
            Json(CreateDraftInput {
                title: "x".repeat(201),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn create_draft_persists_via_repo() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_create_draft()
            .with(eq("u1"), eq("My first post".to_string()))
            .returning(|_, _| Ok(post("p1", "u1", false)));

        let response = create_draft(
            State(repos(posts)),
            AuthUser(claims("u1")),
            Json(CreateDraftInput {
                title: "My first post".into(),
            }),
        )
        .await
        .expect("handler should succeed");

        assert!(response.0.get("post").is_some());
    }

    #[tokio::test]
    async fn publish_forbidden_for_non_author() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "author", false))));

        let result = publish_post(
            State(repos(posts)),
            State(collab()),
            AuthUser(claims("not-author")),
            Path("p1".into()),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn publish_rejects_already_published() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "u1", true))));

        let result = publish_post(
            State(repos(posts)),
            State(collab()),
            AuthUser(claims("u1")),
            Path("p1".into()),
        )
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    fn repos_with(posts: MockPostRepo, social: MockSocialRepo) -> Repos {
        Repos {
            users: Arc::new(MockUserRepo::new()),
            servers: Arc::new(MockServerRepo::new()),
            channels: Arc::new(MockChannelRepo::new()),
            messages: Arc::new(MockMessageRepo::new()),
            social: Arc::new(social),
            posts: Arc::new(posts),
            whiteboards: Arc::new(MockWhiteboardRepo::new()),
            watch: Arc::new(MockWatchRepo::new()),
            recommendations: Arc::new(MockRecommendationsRepo::new()),
        }
    }

    #[tokio::test]
    async fn get_post_allows_author_on_draft() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "u1", false))));
        // is_invited must not be called when the user is the author
        posts.expect_is_invited().never();

        let response = get_post(
            State(repos(posts)),
            AuthUser(claims("u1")),
            Path("p1".into()),
        )
        .await
        .expect("author should be allowed");
        assert!(response.0.get("post").is_some());
    }

    #[tokio::test]
    async fn get_post_blocks_non_collaborator_on_draft() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "author", false))));
        posts.expect_is_invited().returning(|_, _| Ok(false));

        let result = get_post(
            State(repos(posts)),
            AuthUser(claims("stranger")),
            Path("p1".into()),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn get_post_allows_invited_collaborator_on_draft() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "author", false))));
        posts.expect_is_invited().returning(|_, _| Ok(true));

        let response = get_post(
            State(repos(posts)),
            AuthUser(claims("collab")),
            Path("p1".into()),
        )
        .await
        .expect("invited collaborator should be allowed");
        assert!(response.0.get("post").is_some());
    }

    #[tokio::test]
    async fn get_post_public_for_published() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "author", true))));
        posts.expect_is_invited().never();

        let response = get_post(
            State(repos(posts)),
            AuthUser(claims("stranger")),
            Path("p1".into()),
        )
        .await
        .expect("published posts should be public");
        assert!(response.0.get("post").is_some());
    }

    #[tokio::test]
    async fn invite_rejects_non_author() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "author", false))));

        let result = invite_collaborator(
            State(repos(posts)),
            AuthUser(claims("not-author")),
            Path("p1".into()),
            Json(InviteInput {
                user_id: "friend".into(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn invite_rejects_published_post() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "u1", true))));

        let result = invite_collaborator(
            State(repos(posts)),
            AuthUser(claims("u1")),
            Path("p1".into()),
            Json(InviteInput {
                user_id: "friend".into(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn invite_rejects_self_invite() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "u1", false))));

        let result = invite_collaborator(
            State(repos(posts)),
            AuthUser(claims("u1")),
            Path("p1".into()),
            Json(InviteInput {
                user_id: "u1".into(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn invite_rejects_non_friend() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "u1", false))));
        let mut social = MockSocialRepo::new();
        social.expect_are_friends().returning(|_, _| Ok(false));

        let result = invite_collaborator(
            State(repos_with(posts, social)),
            AuthUser(claims("u1")),
            Path("p1".into()),
            Json(InviteInput {
                user_id: "stranger".into(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn invite_succeeds_for_friend() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "u1", false))));
        posts
            .expect_add_invite()
            .with(eq("p1"), eq("friend"))
            .returning(|_, _| Ok(()));
        let mut social = MockSocialRepo::new();
        social
            .expect_are_friends()
            .with(eq("u1"), eq("friend"))
            .returning(|_, _| Ok(true));

        let response = invite_collaborator(
            State(repos_with(posts, social)),
            AuthUser(claims("u1")),
            Path("p1".into()),
            Json(InviteInput {
                user_id: "friend".into(),
            }),
        )
        .await
        .expect("invite should succeed");
        assert_eq!(response.0["ok"], json!(true));
    }

    #[tokio::test]
    async fn publish_freezes_empty_content_on_empty_doc() {
        let mut posts = MockPostRepo::new();
        posts
            .expect_find_by_id()
            .returning(|_| Ok(Some(post("p1", "u1", false))));
        posts
            .expect_publish()
            .with(eq("p1"), eq(String::new()))
            .returning(|_, _| Ok(post("p1", "u1", true)));

        let response = publish_post(
            State(repos(posts)),
            State(collab()),
            AuthUser(claims("u1")),
            Path("p1".into()),
        )
        .await
        .expect("handler should succeed");

        let published = response.0["post"]["published"].as_bool();
        assert_eq!(published, Some(true));
    }
}
