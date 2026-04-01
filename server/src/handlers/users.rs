use axum::extract::Path;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::models::user::CreateUser;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

pub async fn create_user(
    Json(_input): Json<CreateUser>,
) -> Result<Json<Value>, AppError> {
    // TODO: Hash password, insert user into SurrealDB, return user record
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}

pub async fn get_user(
    Path(_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // TODO: Query user by ID from SurrealDB
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}

pub async fn login(
    Json(_input): Json<LoginRequest>,
) -> Result<Json<Value>, AppError> {
    // TODO: Verify credentials, return JWT token
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}
