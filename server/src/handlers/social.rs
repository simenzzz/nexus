use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::AppState;

// ── Friend requests ──

#[derive(Debug, Deserialize)]
pub struct FriendRequestInput {
    pub user_id: String,
}

pub async fn send_friend_request(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(input): Json<FriendRequestInput>,
) -> Result<Json<Value>, AppError> {
    state
        .repos
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
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(input): Json<AcceptRequestInput>,
) -> Result<Json<Value>, AppError> {
    state
        .repos
        .social
        .accept_friend_request(&input.user_id, &claims.sub)
        .await?;

    Ok(Json(json!({ "status": "accepted" })))
}

pub async fn remove_friend(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    state
        .repos
        .social
        .remove_friend(&claims.sub, &user_id)
        .await?;

    Ok(Json(json!({ "status": "removed" })))
}

pub async fn list_friends(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let friends = state.repos.social.list_friends(&claims.sub).await?;

    Ok(Json(json!({ "friends": friends })))
}

pub async fn list_pending_incoming(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let pending = state.repos.social.list_pending_incoming(&claims.sub).await?;

    Ok(Json(json!({ "pending": pending })))
}

pub async fn get_mutual_friends(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mutual = state
        .repos
        .social
        .get_mutual_friends(&claims.sub, &user_id)
        .await?;

    Ok(Json(json!({ "mutual_friends": mutual })))
}

pub async fn get_friend_suggestions(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let suggestions = state.repos.social.get_friend_suggestions(&claims.sub, 20).await?;

    Ok(Json(json!({ "suggestions": suggestions })))
}

// ── Follow / Unfollow ──

pub async fn follow_user(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.repos.social.follow(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "following" })))
}

pub async fn unfollow_user(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.repos.social.unfollow(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "unfollowed" })))
}

// ── Block / Unblock ──

pub async fn block_user(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.repos.social.block_user(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "blocked" })))
}

pub async fn unblock_user(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.repos.social.unblock_user(&claims.sub, &user_id).await?;

    Ok(Json(json!({ "status": "unblocked" })))
}
