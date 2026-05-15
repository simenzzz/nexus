use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::models::channel::{ChannelType, CreateChannel};
use crate::models::server::CreateServer;
use crate::AppState;

/// Extract the key portion from a SurrealDB RecordId string ("table:key" → "key")
fn extract_record_key(record_id: &surrealdb::RecordId) -> String {
    let s = record_id.to_string();
    s.split_once(':').map(|(_, k)| k.to_string()).unwrap_or(s)
}

pub async fn create_server(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(input): Json<CreateServer>,
) -> Result<Json<Value>, AppError> {
    // Validate server name
    let name = input.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::BadRequest("Server name must be 1-100 characters".into()));
    }

    let server = state.repos.servers.create(input, &claims.sub).await?;

    let server_id = server
        .id
        .as_ref()
        .ok_or_else(|| AppError::Internal("Server created without ID".into()))
        .map(extract_record_key)?;

    // Auto-add owner as member
    state.repos.servers.add_member(&server_id, &claims.sub).await?;

    // Auto-create #general channel
    let general = CreateChannel {
        name: "general".to_string(),
        channel_type: ChannelType::Text,
    };
    state.repos.channels.create(general, &server_id).await?;

    Ok(Json(json!({
        "server": server,
    })))
}

pub async fn get_server(
    State(state): State<AppState>,
    AuthUser(_claims): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let server = state
        .repos
        .servers
        .find_by_id(&id)
        .await?
        .ok_or_else(|| AppError::NotFound("Server not found".into()))?;

    Ok(Json(json!({ "server": server })))
}

pub async fn list_servers(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let servers = state.repos.servers.list_for_user(&claims.sub).await?;

    Ok(Json(json!({ "servers": servers })))
}

pub async fn join_server(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // Verify server exists
    state
        .repos
        .servers
        .find_by_id(&id)
        .await?
        .ok_or_else(|| AppError::NotFound("Server not found".into()))?;

    state.repos.servers.add_member(&id, &claims.sub).await?;

    Ok(Json(json!({ "status": "joined" })))
}
