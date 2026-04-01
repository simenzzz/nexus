use axum::extract::Path;
use axum::Json;
use serde_json::Value;

use crate::error::AppError;

pub async fn get_messages(
    Path(_channel_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // TODO: Query message history with pagination (before cursor + limit)
    Ok(Json(serde_json::json!({ "status": "not implemented" })))
}
