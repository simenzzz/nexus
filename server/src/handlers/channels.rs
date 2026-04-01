use axum::extract::Path;
use axum::Json;
use serde_json::Value;

use crate::error::AppError;
use crate::models::channel::CreateChannel;

pub async fn create_channel(
    Path(_server_id): Path<String>,
    Json(_input): Json<CreateChannel>,
) -> Result<Json<Value>, AppError> {
    // TODO: Insert channel into SurrealDB under the given server
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}

pub async fn get_channels(
    Path(_server_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // TODO: List channels for the given server
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}
