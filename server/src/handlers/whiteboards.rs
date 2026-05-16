use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::collab::resource::ResourceRef;
use crate::collab::CollabManager;
use crate::error::AppError;
use crate::models::channel::ChannelType;
use crate::repositories::Repos;

#[derive(Debug, Deserialize)]
pub struct CreateCheckpointInput {
    /// Optional human label (e.g. "before redesign"). Stored as-is.
    pub label: Option<String>,
}

/// `GET /api/channels/:channel_id/whiteboard` — current snapshot + metadata.
/// Returns `{ state_b64: "", state_vector_b64: "" }` for never-edited
/// whiteboards so the client can still bootstrap a blank Y.Doc.
pub async fn get_whiteboard(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    authorize_member(&repos, &channel_id, &claims.sub).await?;

    let wb = repos.whiteboards.find_by_channel(&channel_id).await?;
    let (state_b64, state_vector_b64, snapshot_count) = match wb {
        Some(w) => (w.state_b64, w.state_vector_b64, w.snapshot_count),
        None => (String::new(), String::new(), 0),
    };
    Ok(Json(json!({
        "channel_id": channel_id,
        "state_b64": state_b64,
        "state_vector_b64": state_vector_b64,
        "snapshot_count": snapshot_count,
    })))
}

/// `POST /api/channels/:channel_id/whiteboard/checkpoints` — capture a manual
/// checkpoint at the current state. Authorized to any server member.
pub async fn create_checkpoint(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(channel_id): Path<String>,
    Json(input): Json<CreateCheckpointInput>,
) -> Result<Json<Value>, AppError> {
    authorize_member(&repos, &channel_id, &claims.sub).await?;

    let wb = repos
        .whiteboards
        .find_by_channel(&channel_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Whiteboard has no state yet".into()))?;

    let checkpoint = repos
        .whiteboards
        .append_checkpoint(&channel_id, wb.state_b64, input.label)
        .await?;

    Ok(Json(json!({ "checkpoint": checkpoint })))
}

/// `GET /api/channels/:channel_id/whiteboard/checkpoints` — list saved
/// checkpoints, newest first. Capped server-side at MAX_CHECKPOINTS.
pub async fn list_checkpoints(
    State(repos): State<Repos>,
    AuthUser(claims): AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    authorize_member(&repos, &channel_id, &claims.sub).await?;
    let rows = repos.whiteboards.list_checkpoints(&channel_id).await?;
    Ok(Json(json!({ "checkpoints": rows })))
}

/// `POST /api/channels/:channel_id/whiteboard/checkpoints/:checkpoint_id/restore`
/// Replaces the live whiteboard state with the snapshot from the named
/// checkpoint and notifies any active editors so they re-subscribe.
pub async fn restore_checkpoint(
    State(repos): State<Repos>,
    State(collab): State<CollabManager>,
    AuthUser(claims): AuthUser,
    Path((channel_id, checkpoint_id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    authorize_member(&repos, &channel_id, &claims.sub).await?;

    let checkpoint = repos
        .whiteboards
        .find_checkpoint(&checkpoint_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Checkpoint not found".into()))?;

    // Belt-and-braces: a checkpoint belongs to one channel; reject if the
    // path tries to attach a foreign checkpoint to a different whiteboard.
    let cp_channel = checkpoint.channel.key().to_string();
    if cp_channel != channel_id {
        return Err(AppError::Forbidden(
            "Checkpoint does not belong to this whiteboard".into(),
        ));
    }

    // Close the live session FIRST so any in-flight edits that land between
    // the DB write and the eviction get applied to the about-to-be-discarded
    // session (and bounced via the broadcast) rather than racing the
    // newly-restored snapshot. Order matters: close → upsert → broadcast.
    // The `WhiteboardClosed` broadcast forces clients to re-subscribe, at
    // which point they pull the restored bytes.
    let r = ResourceRef::whiteboard(channel_id.clone());
    if let Err(e) = collab.close(&r, "restored").await {
        tracing::warn!(channel_id = %channel_id, error = %e, "close before restore failed");
    }

    // Overwrite the live snapshot. state_vector_b64 left blank — clients
    // will recompute it from the restored full state on next subscribe (the
    // manager's subscribe path re-encodes from the in-memory doc).
    repos
        .whiteboards
        .upsert_snapshot(&channel_id, checkpoint.state_b64, String::new())
        .await?;

    Ok(Json(json!({
        "ok": true,
        "channel_id": channel_id,
        "restored_from": checkpoint_id,
    })))
}

/// Membership + whiteboard-type check. **All failure modes (missing channel,
/// wrong channel type, non-member) collapse to a single `Forbidden` so the
/// endpoint can't be used to enumerate channel IDs or distinguish text vs
/// whiteboard channels from outside the server.** Detailed reasons are
/// logged server-side for debugging.
async fn authorize_member(repos: &Repos, channel_id: &str, user_id: &str) -> Result<(), AppError> {
    let channel = match repos.channels.find_by_id(channel_id).await? {
        Some(c) => c,
        None => {
            tracing::debug!(channel_id = %channel_id, user_id = %user_id, "auth: channel missing");
            return Err(AppError::Forbidden("Not authorized for this whiteboard".into()));
        }
    };

    if !matches!(channel.channel_type, ChannelType::Whiteboard) {
        tracing::debug!(channel_id = %channel_id, user_id = %user_id, "auth: wrong channel type");
        return Err(AppError::Forbidden("Not authorized for this whiteboard".into()));
    }

    let server_key = channel.server.key().to_string();
    let is_member = repos.servers.is_member(&server_key, user_id).await?;
    if !is_member {
        tracing::debug!(channel_id = %channel_id, user_id = %user_id, "auth: non-member");
        return Err(AppError::Forbidden("Not authorized for this whiteboard".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::Claims;
    use crate::models::channel::Channel;
    use crate::models::whiteboard::{Whiteboard, WhiteboardCheckpoint};
    use crate::repositories::{
        channel::MockChannelRepo, message::MockMessageRepo, post::MockPostRepo,
        server::MockServerRepo, social::MockSocialRepo, user::MockUserRepo,
        whiteboard::MockWhiteboardRepo,
    };
    use std::sync::Arc;

    fn claims(user_id: &str) -> Claims {
        Claims {
            sub: user_id.into(),
            token_type: "access".into(),
            iat: 0,
            exp: usize::MAX / 2,
        }
    }

    fn channel(id: &str, server_id: &str, kind: ChannelType) -> Channel {
        Channel {
            id: Some(surrealdb::RecordId::from(("channel", id))),
            name: "wb".into(),
            channel_type: kind,
            server: surrealdb::RecordId::from(("server", server_id)),
            created_at: None,
        }
    }

    fn whiteboard(channel_id: &str, state: &str, count: u64) -> Whiteboard {
        Whiteboard {
            id: Some(surrealdb::RecordId::from(("whiteboard", channel_id))),
            channel: surrealdb::RecordId::from(("channel", channel_id)),
            state_b64: state.into(),
            state_vector_b64: String::new(),
            snapshot_count: count,
            last_snapshot_at: None,
            created_at: None,
        }
    }

    fn build_repos(
        channels: MockChannelRepo,
        servers: MockServerRepo,
        whiteboards: MockWhiteboardRepo,
    ) -> Repos {
        Repos {
            users: Arc::new(MockUserRepo::new()),
            servers: Arc::new(servers),
            channels: Arc::new(channels),
            messages: Arc::new(MockMessageRepo::new()),
            social: Arc::new(MockSocialRepo::new()),
            posts: Arc::new(MockPostRepo::new()),
            whiteboards: Arc::new(whiteboards),
        }
    }

    fn collab_empty() -> CollabManager {
        CollabManager::new(Arc::new(MockPostRepo::new()))
    }

    #[tokio::test]
    async fn get_whiteboard_returns_empty_for_fresh_channel() {
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));
        let mut wbs = MockWhiteboardRepo::new();
        wbs.expect_find_by_channel().returning(|_| Ok(None));

        let resp = get_whiteboard(
            State(build_repos(chans, servers, wbs)),
            AuthUser(claims("u1")),
            Path("c1".into()),
        )
        .await
        .expect("ok");
        assert_eq!(resp.0["state_b64"].as_str(), Some(""));
        assert_eq!(resp.0["snapshot_count"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn get_whiteboard_rejects_non_member() {
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(false));
        let wbs = MockWhiteboardRepo::new();

        let result = get_whiteboard(
            State(build_repos(chans, servers, wbs)),
            AuthUser(claims("stranger")),
            Path("c1".into()),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn get_whiteboard_rejects_wrong_channel_type() {
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Text))));
        let servers = MockServerRepo::new();
        let wbs = MockWhiteboardRepo::new();

        let result = get_whiteboard(
            State(build_repos(chans, servers, wbs)),
            AuthUser(claims("u1")),
            Path("c1".into()),
        )
        .await;
        // Collapsed to Forbidden so callers can't enumerate channel types.
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn get_whiteboard_returns_forbidden_for_missing_channel() {
        let mut chans = MockChannelRepo::new();
        chans.expect_find_by_id().returning(|_| Ok(None));
        let servers = MockServerRepo::new();
        let wbs = MockWhiteboardRepo::new();

        let result = get_whiteboard(
            State(build_repos(chans, servers, wbs)),
            AuthUser(claims("u1")),
            Path("c1".into()),
        )
        .await;
        // Missing → Forbidden too (no ID enumeration).
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn create_checkpoint_rejects_uninitialized_whiteboard() {
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));
        let mut wbs = MockWhiteboardRepo::new();
        wbs.expect_find_by_channel().returning(|_| Ok(None));

        let result = create_checkpoint(
            State(build_repos(chans, servers, wbs)),
            AuthUser(claims("u1")),
            Path("c1".into()),
            Json(CreateCheckpointInput { label: None }),
        )
        .await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn restore_checkpoint_rejects_mismatched_channel() {
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));
        let mut wbs = MockWhiteboardRepo::new();
        wbs.expect_find_checkpoint().returning(|_| {
            Ok(Some(WhiteboardCheckpoint {
                id: Some(surrealdb::RecordId::from(("whiteboard_checkpoint", "cp1"))),
                // Checkpoint belongs to a DIFFERENT channel:
                channel: surrealdb::RecordId::from(("channel", "other")),
                state_b64: "x".into(),
                label: None,
                created_at: None,
            }))
        });

        let result = restore_checkpoint(
            State(build_repos(chans, servers, wbs)),
            State(collab_empty()),
            AuthUser(claims("u1")),
            Path(("c1".into(), "cp1".into())),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }

    #[tokio::test]
    async fn restore_checkpoint_overwrites_live_state() {
        let mut chans = MockChannelRepo::new();
        chans
            .expect_find_by_id()
            .returning(|_| Ok(Some(channel("c1", "s1", ChannelType::Whiteboard))));
        let mut servers = MockServerRepo::new();
        servers.expect_is_member().returning(|_, _| Ok(true));
        let mut wbs = MockWhiteboardRepo::new();
        wbs.expect_find_checkpoint().returning(|_| {
            Ok(Some(WhiteboardCheckpoint {
                id: Some(surrealdb::RecordId::from(("whiteboard_checkpoint", "cp1"))),
                channel: surrealdb::RecordId::from(("channel", "c1")),
                state_b64: "FROZEN".into(),
                label: Some("v1".into()),
                created_at: None,
            }))
        });
        wbs.expect_upsert_snapshot()
            .withf(|ch, state, _| ch == "c1" && state == "FROZEN")
            .returning(|_, _, _| Ok(2));

        let response = restore_checkpoint(
            State(build_repos(chans, servers, wbs)),
            State(collab_empty()),
            AuthUser(claims("u1")),
            Path(("c1".into(), "cp1".into())),
        )
        .await
        .expect("restore succeeds");
        assert_eq!(response.0["restored_from"].as_str(), Some("cp1"));
    }

    /// Suppress dead-code warnings on the unused `whiteboard` constructor
    /// (test helper kept for symmetry / future tests).
    #[test]
    fn whiteboard_helper_unused() {
        let _ = whiteboard("c1", "x", 1);
    }
}
