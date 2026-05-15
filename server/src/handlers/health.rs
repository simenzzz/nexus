use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::AppState;

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let output = state.metrics_handle.render();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        output,
    )
}

pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = check_db(&state).await;
    let redis_ok = check_redis(&state).await;

    if db_ok && redis_ok {
        (axum::http::StatusCode::OK, Json(json!({ "status": "ready" })))
    } else {
        let mut details = Vec::new();
        if !db_ok {
            details.push("database");
        }
        if !redis_ok {
            details.push("redis");
        }
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "not_ready",
                "failures": details,
            })),
        )
    }
}

async fn check_db(state: &AppState) -> bool {
    state
        .db
        .query("SELECT 1 AS health")
        .await
        .map(|_| true)
        .unwrap_or(false)
}

async fn check_redis(state: &AppState) -> bool {
    let conn = state.redis.get().await;
    match conn {
        Ok(mut conn) => redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .is_ok(),
        Err(_) => false,
    }
}
