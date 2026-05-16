use std::collections::{HashMap, HashSet};

use axum::extract::{Query, State, WebSocketUpgrade};
use axum::extract::ws::{Message, WebSocket};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::auth::ws_ticket;
use crate::collab::resource::ResourceRef;
use crate::collab::CollabManager;
use crate::middleware::rate_limit::{
    check_rate_limit, collab_subscribe_key, message_send_key, watch_playback_control_key,
    whiteboard_awareness_key, whiteboard_subscribe_key, whiteboard_update_key, RateLimitConfig,
};

/// Cap on the serialized JSON size of an awareness blob. The blob is held in
/// memory per session and broadcast to every peer, so an unbounded blob is a
/// DoS amplification vector independent of the 30/sec rate limit. 4 KB is
/// enough for a cursor + selection + tool color; oversize blobs are rejected.
const MAX_AWARENESS_BYTES: usize = 4 * 1024;
use crate::ws::presence;
use crate::ws::protocol::{ClientMessage, ServerMessage, SubscriptionLevel};
use crate::ws::replay;
use crate::ws::room::RoomCommand;
use crate::ws::sequence;
use crate::ws::watch_types::WatchCommand;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    ticket: Option<String>,
}

pub async fn handle_ws_upgrade(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> Result<Response, Response> {
    let ticket = query.ticket.ok_or_else(|| {
        Response::builder()
            .status(401)
            .body("Missing ticket".into())
            .unwrap_or_default()
    })?;

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, ticket)))
}

async fn handle_socket(socket: WebSocket, state: AppState, ticket: String) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Consume ticket from URL query parameter directly (atomic GETDEL)
    let user_id = match ws_ticket::consume_ticket(&state.redis, &ticket).await {
        Ok(Some(id)) => id,
        _ => {
            let _ = ws_sender
                .send(Message::Text(
                    ServerMessage::Error {
                        message: "Invalid or expired ticket".into(),
                    }
                    .to_json()
                    .into(),
                ))
                .await;
            return;
        }
    };

    // Fetch user profile for real username/avatar
    let (username, avatar_url) = match state.repos.users.find_by_id(&user_id).await {
        Ok(Some(user)) => (user.username, user.avatar_url),
        _ => (user_id.clone(), None),
    };

    // Send auth_ok
    let auth_ok = ServerMessage::AuthOk {
        user_id: user_id.clone(),
        heartbeat_interval: 30000,
    };
    if ws_sender.send(Message::Text(auth_ok.to_json().into())).await.is_err() {
        return;
    }

    // Set user online
    let _ = presence::set_online_with_ttl(&state.redis, &user_id, 300).await;
    crate::metrics::record_ws_connect();

    // Channel for outgoing messages (writer task reads from this)
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);

    // Generate a unique connection ID and register
    let conn_id = uuid::Uuid::new_v4().to_string();
    state.user_connections.register(&user_id, conn_id.clone(), out_tx.clone());

    // Notify friends that user is online
    let friend_ids = state
        .repos
        .social
        .get_friend_ids(&user_id)
        .await
        .unwrap_or_default();

    let online_msg = ServerMessage::Presence {
        user_id: user_id.clone(),
        status: "online".to_string(),
    }
    .to_json();

    for friend_id in &friend_ids {
        if state.user_connections.is_online(friend_id) {
            state
                .user_connections
                .send_to_user(friend_id, online_msg.clone())
                .await;
        }
    }

    // Spawn writer task
    let writer_handle = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Track subscriptions
    let mut subscriptions: HashMap<String, SubscriptionLevel> = HashMap::new();
    let mut collab_subscriptions: HashSet<ResourceRef> = HashSet::new();
    let mut watch_subscriptions: HashSet<String> = HashSet::new();
    let mut last_typing: HashMap<String, std::time::Instant> = HashMap::new();

    let heartbeat_timeout = std::time::Duration::from_secs(60);
    let idle_timeout = std::time::Duration::from_secs(300);

    let mut heartbeat_deadline = tokio::time::Instant::now() + heartbeat_timeout;
    let mut idle_deadline = tokio::time::Instant::now() + idle_timeout;
    let mut is_idle = false;

    // Main read loop — race heartbeat + idle timeouts against incoming messages
    loop {
        let msg = tokio::select! {
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(m)) => m,
                    _ => break, // Connection closed or error
                }
            }
            _ = tokio::time::sleep_until(heartbeat_deadline) => {
                tracing::info!(%user_id, "Heartbeat timeout, disconnecting");
                break;
            }
            _ = tokio::time::sleep_until(idle_deadline) => {
                if !is_idle {
                    is_idle = true;
                    let _ = presence::set_status(&state.redis, &user_id, "idle").await;
                    let idle_msg = ServerMessage::Presence {
                        user_id: user_id.clone(),
                        status: "idle".to_string(),
                    }.to_json();
                    for friend_id in &friend_ids {
                        state.user_connections.send_to_user(friend_id, idle_msg.clone()).await;
                    }
                }
                // Reset idle deadline to keep checking, but don't disconnect
                idle_deadline = tokio::time::Instant::now() + idle_timeout;
                continue;
            }
        };

        match msg {
            Message::Text(text) => {
                let client_msg: ClientMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                match client_msg {
                    ClientMessage::Auth { .. } => {
                        // Already authenticated via URL ticket, ignore
                    }
                    ClientMessage::Heartbeat => {
                        heartbeat_deadline =
                            tokio::time::Instant::now() + heartbeat_timeout;
                        idle_deadline = tokio::time::Instant::now() + idle_timeout;

                        // Restore from idle if needed
                        if is_idle {
                            is_idle = false;
                            let _ = presence::set_online_with_ttl(&state.redis, &user_id, 300).await;
                            let online_msg = ServerMessage::Presence {
                                user_id: user_id.clone(),
                                status: "online".to_string(),
                            }.to_json();
                            for friend_id in &friend_ids {
                                state.user_connections.send_to_user(friend_id, online_msg.clone()).await;
                            }
                        } else {
                            let _ = presence::set_online_with_ttl(&state.redis, &user_id, 300).await;
                        }

                        let _ = out_tx.send(ServerMessage::HeartbeatAck.to_json()).await;
                    }
                    ClientMessage::Subscribe {
                        channel_id,
                        level,
                    } => {
                        // Authorization: verify user is a member of the channel's server
                        if !check_channel_access(&state, &channel_id, &user_id).await {
                            let _ = out_tx
                                .send(
                                    ServerMessage::Error {
                                        message: "Not a member of this server".into(),
                                    }
                                    .to_json(),
                                )
                                .await;
                            continue;
                        }

                        let room = state.room_manager.get_or_create(&channel_id).await;
                        let _ = room
                            .send(RoomCommand::Join {
                                user_id: user_id.clone(),
                                username: username.clone(),
                                level: level.clone(),
                                sender: out_tx.clone(),
                            })
                            .await;
                        subscriptions.insert(channel_id, level);
                    }
                    ClientMessage::Unsubscribe { channel_id } => {
                        if let Some(room) = state.room_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(RoomCommand::Leave {
                                    user_id: user_id.clone(),
                                })
                                .await;
                        }
                        subscriptions.remove(&channel_id);
                    }
                    ClientMessage::ChatMessage {
                        channel_id,
                        content,
                        nonce,
                    } => {
                        // Authorization: must be subscribed to the channel
                        if !subscriptions.contains_key(&channel_id) {
                            let _ = out_tx
                                .send(
                                    ServerMessage::Error {
                                        message: "Not subscribed to this channel".into(),
                                    }
                                    .to_json(),
                                )
                                .await;
                            continue;
                        }

                        // Validate content length
                        if content.is_empty() || content.len() > 4000 {
                            let _ = out_tx
                                .send(
                                    ServerMessage::Error {
                                        message: "Message must be 1-4000 characters".into(),
                                    }
                                    .to_json(),
                                )
                                .await;
                            continue;
                        }

                        // Rate limit: 5 messages per 5 seconds per user per channel
                        let rate_key = message_send_key(&user_id, &channel_id);
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: rate_key,
                                limit: 5,
                                window_secs: 5,
                            },
                        )
                        .await
                        .is_err()
                        {
                            let _ = out_tx
                                .send(
                                    ServerMessage::Error {
                                        message: "Message rate limited".into(),
                                    }
                                    .to_json(),
                                )
                                .await;
                            continue;
                        }

                        let seq = match sequence::next_seq(&state.redis, &channel_id).await {
                            Ok(s) => s,
                            Err(_) => continue,
                        };

                        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
                        let msg_id = uuid::Uuid::new_v4().to_string();

                        let server_msg = ServerMessage::ChatMessage {
                            seq,
                            channel_id: channel_id.clone(),
                            message_id: msg_id.clone(),
                            author: crate::ws::protocol::MessageAuthor {
                                id: user_id.clone(),
                                username: username.clone(),
                                avatar_url: avatar_url.clone(),
                            },
                            content: content.clone(),
                            ts: now_ms,
                        };

                        let _ = replay::store_message(
                            &state.redis,
                            &channel_id,
                            seq,
                            &server_msg.to_json(),
                        )
                        .await;

                        // Persist to SurrealDB (fire-and-forget)
                        let repos = state.repos.clone();
                        let content_clone = content.clone();
                        let user_id_clone = user_id.clone();
                        let channel_id_clone = channel_id.clone();
                        let msg_id_clone = msg_id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = repos
                                .messages
                                .create_with_id(
                                    &msg_id_clone,
                                    content_clone,
                                    &user_id_clone,
                                    &channel_id_clone,
                                )
                                .await
                            {
                                tracing::error!(error = %e, "Failed to persist message to SurrealDB");
                            }
                        });

                        // ACK to sender
                        let ack = ServerMessage::MessageAck {
                            nonce,
                            message_id: msg_id,
                            seq,
                            ts: now_ms,
                        };
                        let _ = out_tx.send(ack.to_json()).await;

                        // Broadcast to room (excluding sender)
                        if let Some(room) = state.room_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(RoomCommand::Broadcast {
                                    message: server_msg.to_json(),
                                    exclude_user: Some(user_id.clone()),
                                })
                                .await;
                        }
                    }
                    ClientMessage::Typing { channel_id } => {
                        if !subscriptions.contains_key(&channel_id) {
                            continue;
                        }

                        let now = std::time::Instant::now();
                        if let Some(last) = last_typing.get(&channel_id) {
                            if now.duration_since(*last).as_secs() < 3 {
                                continue;
                            }
                        }
                        last_typing.insert(channel_id.clone(), now);

                        let typing_msg = ServerMessage::Typing {
                            channel_id: channel_id.clone(),
                            user_id: user_id.clone(),
                            username: username.clone(),
                        };
                        if let Some(room) = state.room_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(RoomCommand::Broadcast {
                                    message: typing_msg.to_json(),
                                    exclude_user: Some(user_id.clone()),
                                })
                                .await;
                        }
                    }
                    ClientMessage::Resume { last_seq } => {
                        for (channel_id, last) in last_seq {
                            // Authorization check before resuming
                            if !check_channel_access(&state, &channel_id, &user_id).await {
                                continue;
                            }

                            // Re-subscribe to the room
                            let room = state.room_manager.get_or_create(&channel_id).await;
                            let _ = room
                                .send(RoomCommand::Join {
                                    user_id: user_id.clone(),
                                    username: username.clone(),
                                    level: SubscriptionLevel::Active,
                                    sender: out_tx.clone(),
                                })
                                .await;
                            subscriptions
                                .insert(channel_id.clone(), SubscriptionLevel::Active);

                            // Replay missed messages
                            match replay::get_missed_messages(&state.redis, &channel_id, last).await
                            {
                                Ok(Some(messages)) => {
                                    for msg in messages {
                                        let _ = out_tx.send(msg).await;
                                    }
                                }
                                Ok(None) => {
                                    let resync = ServerMessage::Resync {
                                        channel_id: channel_id.clone(),
                                    };
                                    let _ = out_tx.send(resync.to_json()).await;
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    // Phase 2 collab + Phase 3 whiteboard messages: route to
                    // CollabManager via a typed ResourceRef.
                    ClientMessage::CollabSubscribe { post_id } => {
                        // Rate-limit subscribe churn so the (uncached) authz
                        // path can't be hammered. 10/sec per user is plenty
                        // for legitimate page navigation.
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: collab_subscribe_key(&user_id),
                                limit: 10,
                                window_secs: 1,
                            },
                        )
                        .await
                        .is_err()
                        {
                            continue;
                        }
                        let r = ResourceRef::post(post_id);
                        collab_subscriptions.insert(r.clone());
                        if let Err(e) = state
                            .collab
                            .subscribe(&r, &user_id, out_tx.clone())
                            .await
                        {
                            CollabManager::send_error(&out_tx, &r, "subscribe_failed", &e).await;
                        }
                    }
                    ClientMessage::CollabUnsubscribe { post_id } => {
                        let r = ResourceRef::post(post_id);
                        collab_subscriptions.remove(&r);
                        state.collab.unsubscribe(&r, &user_id).await;
                    }
                    ClientMessage::CollabUpdate { post_id, update_b64 } => {
                        let r = ResourceRef::post(post_id);
                        if let Err(e) = state
                            .collab
                            .apply_update(&r, &user_id, &update_b64)
                            .await
                        {
                            CollabManager::send_error(&out_tx, &r, "update_failed", &e).await;
                        }
                    }
                    ClientMessage::AwarenessUpdate { post_id, state: aw_state } => {
                        if awareness_too_large(&aw_state) {
                            continue;
                        }
                        let r = ResourceRef::post(post_id);
                        state.collab.update_awareness(&r, &user_id, aw_state).await;
                    }
                    ClientMessage::WhiteboardSubscribe { whiteboard_id } => {
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: whiteboard_subscribe_key(&user_id),
                                limit: 10,
                                window_secs: 1,
                            },
                        )
                        .await
                        .is_err()
                        {
                            continue;
                        }
                        let r = ResourceRef::whiteboard(whiteboard_id);
                        collab_subscriptions.insert(r.clone());
                        if let Err(e) = state
                            .collab
                            .subscribe(&r, &user_id, out_tx.clone())
                            .await
                        {
                            CollabManager::send_error(&out_tx, &r, "subscribe_failed", &e).await;
                        }
                    }
                    ClientMessage::WhiteboardUnsubscribe { whiteboard_id } => {
                        let r = ResourceRef::whiteboard(whiteboard_id);
                        collab_subscriptions.remove(&r);
                        state.collab.unsubscribe(&r, &user_id).await;
                    }
                    ClientMessage::WhiteboardUpdate {
                        whiteboard_id,
                        update_b64,
                    } => {
                        let r = ResourceRef::whiteboard(whiteboard_id);
                        // Rate cap: 30 stroke-updates/sec per user per
                        // whiteboard. Live drawing must stay smooth; this
                        // only catches malicious or runaway clients.
                        let rate_key = whiteboard_update_key(&user_id, &r.id);
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: rate_key,
                                limit: 30,
                                window_secs: 1,
                            },
                        )
                        .await
                        .is_err()
                        {
                            CollabManager::send_error(
                                &out_tx,
                                &r,
                                "rate_limited",
                                "Too many updates",
                            )
                            .await;
                            continue;
                        }
                        if let Err(e) = state
                            .collab
                            .apply_update(&r, &user_id, &update_b64)
                            .await
                        {
                            CollabManager::send_error(&out_tx, &r, "update_failed", &e).await;
                        }
                    }
                    ClientMessage::WhiteboardAwarenessUpdate {
                        whiteboard_id,
                        state: aw_state,
                    } => {
                        // Size cap independent of rate limit — an awareness
                        // blob is held per-user in memory and amplified to
                        // every peer on broadcast, so an unbounded JSON
                        // payload is a DoS amplifier even at 30/sec.
                        if awareness_too_large(&aw_state) {
                            continue;
                        }
                        let r = ResourceRef::whiteboard(whiteboard_id);
                        let rate_key = whiteboard_awareness_key(&user_id, &r.id);
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: rate_key,
                                limit: 30,
                                window_secs: 1,
                            },
                        )
                        .await
                        .is_err()
                        {
                            continue;
                        }
                        state.collab.update_awareness(&r, &user_id, aw_state).await;
                    }
                    // ── Phase 4: watch-together rooms ──
                    ClientMessage::WatchSubscribe { channel_id } => {
                        // Defense in depth: verify the channel is actually a
                        // Watch channel AND the user is a server member. The
                        // frontend routes by type, but never trust the client.
                        if !check_watch_channel_access(&state, &channel_id, &user_id).await {
                            let _ = out_tx
                                .send(
                                    ServerMessage::WatchError {
                                        channel_id: channel_id.clone(),
                                        code: "forbidden".into(),
                                        message: "Not a watch channel or not a member".into(),
                                    }
                                    .to_json(),
                                )
                                .await;
                            continue;
                        }
                        let room = state.watch_manager.get_or_create(&channel_id).await;
                        let _ = room
                            .send(WatchCommand::Join {
                                user_id: user_id.clone(),
                                username: username.clone(),
                                sender: out_tx.clone(),
                            })
                            .await;
                        watch_subscriptions.insert(channel_id);
                    }
                    ClientMessage::WatchUnsubscribe { channel_id } => {
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::Leave {
                                    user_id: user_id.clone(),
                                })
                                .await;
                        }
                        watch_subscriptions.remove(&channel_id);
                    }
                    ClientMessage::WatchTransferLeader {
                        channel_id,
                        to_user_id,
                    } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::TransferLeader {
                                    from_user: user_id.clone(),
                                    to_user: to_user_id,
                                    reply_to: out_tx.clone(),
                                })
                                .await;
                        }
                    }
                    ClientMessage::WatchPlayback {
                        channel_id,
                        action,
                        position_ms,
                        client_ts: _,
                    } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        // Rate cap: 10/sec per user per room. Leader is one
                        // user so this is a per-leader cap. Defense against
                        // a runaway client looping seek events.
                        let rate_key = watch_playback_control_key(&user_id, &channel_id);
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: rate_key,
                                limit: 10,
                                window_secs: 1,
                            },
                        )
                        .await
                        .is_err()
                        {
                            let _ = out_tx
                                .send(
                                    ServerMessage::WatchError {
                                        channel_id,
                                        code: "rate_limited".into(),
                                        message: "Playback control rate limited".into(),
                                    }
                                    .to_json(),
                                )
                                .await;
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::PlaybackControl {
                                    from_user: user_id.clone(),
                                    action,
                                    position_ms,
                                    reply_to: out_tx.clone(),
                                })
                                .await;
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Cleanup
    let _ = presence::set_offline(&state.redis, &user_id).await;
    crate::metrics::record_ws_disconnect();

    // Notify friends that user is offline
    let offline_msg = ServerMessage::Presence {
        user_id: user_id.clone(),
        status: "offline".to_string(),
    }
    .to_json();
    for friend_id in &friend_ids {
        state
            .user_connections
            .send_to_user(friend_id, offline_msg.clone())
            .await;
    }

    // Unregister from connection registry
    state.user_connections.unregister(&user_id, &conn_id);

    for channel_id in subscriptions.keys() {
        if let Some(room) = state.room_manager.get_room(channel_id).await {
            let _ = room
                .send(RoomCommand::Leave {
                    user_id: user_id.clone(),
                })
                .await;
        }
    }

    // Tear down any dangling collab/whiteboard subscriptions — without this
    // the CollabManager session holds a dead `mpsc::Sender` and never evicts.
    for r in &collab_subscriptions {
        state.collab.unsubscribe(r, &user_id).await;
    }

    // Same for watch rooms: drop the actor's reference so it can leader-handoff
    // and eventually grace-period-evict if the room is now empty.
    for channel_id in &watch_subscriptions {
        if let Some(room) = state.watch_manager.get_room(channel_id).await {
            let _ = room
                .send(WatchCommand::Leave {
                    user_id: user_id.clone(),
                })
                .await;
        }
    }

    drop(out_tx);
    let _ = writer_handle.await;
    tracing::info!(%user_id, "WebSocket client disconnected");
}

/// Cheap size guard for awareness payloads. Serializes the JSON and rejects
/// anything over [`MAX_AWARENESS_BYTES`]. We compare the *serialized* form so
/// the cap matches what we'd actually hold in memory and broadcast.
fn awareness_too_large(value: &serde_json::Value) -> bool {
    // `serde_json::to_string` allocates — for the common-case small blob
    // this is a few hundred bytes. If it grows we can switch to a streaming
    // writer that aborts at the cap.
    match serde_json::to_string(value) {
        Ok(s) => s.len() > MAX_AWARENESS_BYTES,
        Err(_) => true, // unserializable → reject
    }
}

/// Check if a user has access to a channel by verifying server membership.
async fn check_channel_access(state: &AppState, channel_id: &str, user_id: &str) -> bool {
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

/// Symmetric "not subscribed" surface for the watch protocol — every
/// mutating arm uses this so clients can distinguish "I dropped the message"
/// from "the server silently ignored me." Tracing-only on the server side.
async fn send_watch_not_subscribed(out_tx: &mpsc::Sender<String>, channel_id: &str) {
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

/// Watch-channel-specific access check: in addition to the membership rule,
/// the channel must be `ChannelType::Watch`. Defends against a client trying
/// to multiplex watch protocol commands onto an unrelated channel id.
async fn check_watch_channel_access(state: &AppState, channel_id: &str, user_id: &str) -> bool {
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
