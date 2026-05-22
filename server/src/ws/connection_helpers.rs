use std::collections::HashSet;

use tokio::sync::mpsc;

use crate::middleware::rate_limit::{check_rate_limit, watch_queue_op_key, RateLimitConfig};
use crate::ws::protocol::ServerMessage;
use crate::AppState;

/// Maximum age of a cached presence audience before we re-fetch the graph.
/// Bounds the staleness window for block / unfriend / server-leave changes
/// while a session stays connected.
pub(super) const AUDIENCE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Re-resolve the audience if the cached value is older than [`AUDIENCE_TTL`].
pub(super) async fn refresh_audience_if_stale(
    state: &AppState,
    user_id: &str,
    audience: &mut Vec<String>,
    fetched_at: &mut std::time::Instant,
) {
    if fetched_at.elapsed() <= AUDIENCE_TTL {
        return;
    }
    *audience = compute_presence_audience(state, user_id).await;
    *fetched_at = std::time::Instant::now();
}

/// Build this user's presence audience as the de-duplicated union of their
/// accepted friends and the other members of any server they belong to.
/// Failures on either repo call degrade silently to whatever the other side
/// returned (or empty).
pub(super) async fn compute_presence_audience(state: &AppState, user_id: &str) -> Vec<String> {
    let friends = state
        .repos
        .social
        .get_friend_ids(user_id)
        .await
        .unwrap_or_default();
    let co_members = state
        .repos
        .servers
        .list_co_member_ids(user_id)
        .await
        .unwrap_or_default();
    let mut set: HashSet<String> = friends.into_iter().collect();
    set.extend(co_members);
    set.into_iter().collect()
}

/// Cheap size guard for awareness payloads. Serializes the JSON and rejects
/// anything over the supplied cap.
pub(super) fn awareness_too_large(value: &serde_json::Value, max_bytes: usize) -> bool {
    match serde_json::to_string(value) {
        Ok(s) => s.len() > max_bytes,
        Err(_) => true,
    }
}

/// Check if a user has access to a channel by verifying server membership.
pub(super) async fn check_channel_access(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> bool {
    let channel = match state.repos.channels.find_by_id(channel_id).await {
        Ok(Some(ch)) => ch,
        _ => return false,
    };
    let server_key = channel.server.key().to_string();
    state
        .repos
        .servers
        .is_member(&server_key, user_id)
        .await
        .unwrap_or(false)
}

/// Watch-channel-specific access check: in addition to membership, the
/// channel must be `ChannelType::Watch`. Defends against a client trying to
/// multiplex watch protocol commands onto an unrelated channel id.
pub(super) async fn check_watch_channel_access(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> bool {
    use crate::models::channel::ChannelType;
    let channel = match state.repos.channels.find_by_id(channel_id).await {
        Ok(Some(ch)) => ch,
        _ => return false,
    };
    if channel.channel_type != ChannelType::Watch {
        return false;
    }
    let server_key = channel.server.key().to_string();
    state
        .repos
        .servers
        .is_member(&server_key, user_id)
        .await
        .unwrap_or(false)
}

/// Symmetric "not subscribed" surface for the watch protocol.
pub(super) async fn send_watch_not_subscribed(out_tx: &mpsc::Sender<String>, channel_id: &str) {
    let _ = out_tx
        .send(
            ServerMessage::WatchError {
                channel_id: channel_id.to_string(),
                code: "not_subscribed".into(),
                message: "Not subscribed to this watch room".into(),
            }
            .to_json(),
        )
        .await;
}

/// Apply the per-user-per-room queue-op rate limit. On rejection, surfaces a
/// `watch_error{code:"rate_limited"}` so the optimistic client can roll its
/// pending entry back. Returns `true` if the caller may proceed.
pub(super) async fn check_watch_queue_rate(
    state: &AppState,
    user_id: &str,
    channel_id: &str,
    out_tx: &mpsc::Sender<String>,
) -> bool {
    let key = watch_queue_op_key(user_id, channel_id);
    if check_rate_limit(
        &state.redis,
        &RateLimitConfig {
            key_prefix: key,
            limit: 10,
            window_secs: 60,
        },
    )
    .await
    .is_err()
    {
        let _ = out_tx
            .send(
                ServerMessage::WatchError {
                    channel_id: channel_id.to_string(),
                    code: "rate_limited".into(),
                    message: "Queue operation rate limited".into(),
                }
                .to_json(),
            )
            .await;
        return false;
    }
    true
}
