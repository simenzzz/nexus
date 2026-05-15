use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::AppState;

/// Discover servers ranked by friend-member overlap.
/// Traverses: user -> friends_with -> user -> member_of -> server
pub async fn discover_servers(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Result<Json<Value>, AppError> {
    let limit = 20u32;

    // Get friend IDs
    let friend_ids = state.repos.social.get_friend_ids(&claims.sub).await?;

    if friend_ids.is_empty() {
        // No friends — return recent servers the user hasn't joined
        let mut result = state
            .db
            .query(
                "SELECT * FROM server WHERE id NOT IN \
                 (SELECT VALUE out FROM member_of WHERE in = $user) \
                 ORDER BY created_at DESC LIMIT $limit",
            )
            .bind(("user", surrealdb::RecordId::from(("user", &claims.sub))))
            .bind(("limit", limit))
            .await?;
        let servers: Vec<serde_json::Value> = result.take(0)?;
        return Ok(Json(json!({ "servers": servers })));
    }

    let friend_record_ids: Vec<surrealdb::RecordId> = friend_ids
        .iter()
        .map(|id| surrealdb::RecordId::from(("user", id.as_str())))
        .collect();

    // Find servers where friends are members, ranked by friend overlap
    // In RELATE user -> member_of -> server: in=user, out=server
    let mut result = state
        .db
        .query(
            "SELECT out AS server, count() AS friend_count \
             FROM member_of WHERE in IN $friends \
             AND out NOT IN (SELECT VALUE out FROM member_of WHERE in = $user) \
             GROUP BY server \
             ORDER BY friend_count DESC \
             LIMIT $limit",
        )
        .bind(("friends", friend_record_ids))
        .bind(("user", surrealdb::RecordId::from(("user", &claims.sub))))
        .bind(("limit", limit))
        .await?;

    let servers: Vec<serde_json::Value> = result.take(0)?;

    Ok(Json(json!({ "servers": servers })))
}
