use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub before: Option<String>,
    pub limit: Option<u32>,
}

pub async fn get_messages(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(channel_id): Path<String>,
    Query(params): Query<ListMessagesQuery>,
) -> Result<Json<Value>, AppError> {
    // Verify user has access to this channel's server
    let channel = state
        .repos
        .channels
        .find_by_id(&channel_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;

    let server_key = channel.server.key().to_string();
    if !state.repos.servers.is_member(&server_key, &claims.sub).await? {
        return Err(AppError::Forbidden("Not a member of this server".into()));
    }

    let limit = params.limit.unwrap_or(50).min(100);
    let mut messages = state
        .repos
        .messages
        .list_for_channel(&channel_id, params.before.as_deref(), limit)
        .await?;

    // Repository returns DESC (newest first); reverse to ASC for display
    messages.reverse();

    Ok(Json(json!({ "messages": messages })))
}
