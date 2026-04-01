use axum::extract::Path;
use axum::Json;
use serde_json::Value;

use crate::error::AppError;
use crate::models::server::CreateServer;

pub async fn create_server(
    Json(_input): Json<CreateServer>,
) -> Result<Json<Value>, AppError> {
    // TODO: Insert server into SurrealDB, create default #general channel
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}

pub async fn get_server(
    Path(_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // TODO: Query server by ID from SurrealDB
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}

pub async fn list_servers() -> Result<Json<Value>, AppError> {
    // TODO: List servers the authenticated user is a member of
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}

pub async fn join_server(
    Path(_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // TODO: RELATE user -> member_of -> server
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}
