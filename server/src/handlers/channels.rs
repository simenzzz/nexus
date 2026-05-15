use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::models::channel::CreateChannel;
use crate::AppState;

pub async fn create_channel(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(server_id): Path<String>,
    Json(input): Json<CreateChannel>,
) -> Result<Json<Value>, AppError> {
    // Verify user is the server owner
    let server = state
        .repos
        .servers
        .find_by_id(&server_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Server not found".into()))?;

    let owner_key = server.owner.key().to_string();
    if owner_key != claims.sub {
        return Err(AppError::Forbidden("Only the server owner can create channels".into()));
    }

    // Validate channel name
    let name = input.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::BadRequest(
            "Channel name must be 1-100 characters".into(),
        ));
    }

    let channel = state.repos.channels.create(input, &server_id).await?;

    Ok(Json(json!({ "channel": channel })))
}

pub async fn get_channels(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(server_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // Verify user is a member of the server
    if !state.repos.servers.is_member(&server_id, &claims.sub).await? {
        return Err(AppError::Forbidden("Not a member of this server".into()));
    }

    let channels = state
        .repos
        .channels
        .list_for_server(&server_id)
        .await?;

    Ok(Json(json!({ "channels": channels })))
}
