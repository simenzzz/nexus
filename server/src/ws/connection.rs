use std::collections::{HashMap, HashSet};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::auth::ws_ticket;
use crate::collab::resource::ResourceRef;
use crate::collab::CollabManager;
use crate::middleware::rate_limit::{
    check_rate_limit, collab_subscribe_key, message_send_key, watch_playback_control_key,
    watch_reaction_key, whiteboard_awareness_key, whiteboard_subscribe_key, whiteboard_update_key,
    RateLimitConfig,
};
use crate::ws::connection_helpers::{
    awareness_too_large as awareness_too_large_inner, check_channel_access,
    check_watch_channel_access, check_watch_queue_rate, compute_presence_audience,
    refresh_audience_if_stale, send_watch_not_subscribed,
};

fn awareness_too_large(value: &serde_json::Value) -> bool {
    awareness_too_large_inner(value, MAX_AWARENESS_BYTES)
}

/// Cap on the serialized JSON size of an awareness blob. The blob is held in
/// memory per session and broadcast to every peer, so an unbounded blob is a
/// DoS amplification vector independent of the rate limit. 4 KB is enough for
/// a cursor + selection + tool color; oversize blobs are rejected.
const MAX_AWARENESS_BYTES: usize = 4 * 1024;

/// Watch-queue title hard cap. Enforced at the connection boundary so we
/// fail fast with a typed error instead of silently truncating in the room.
const MAX_WATCH_TITLE_LEN: usize = 200;

/// Server-side min-interval between successive heartbeats from one
/// connection. The client claims it heartbeats every 30s; anything tighter
/// than 2s is either a bug or a probe.
const MIN_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
use crate::ws::presence;
use crate::ws::protocol::{ClientMessage, ServerMessage, SubscriptionLevel};
use crate::ws::replay;
use crate::ws::room::RoomCommand;
use crate::ws::sequence;
use crate::ws::watch_types::WatchCommand;
use crate::AppState;

pub async fn handle_ws_upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, Response> {
    if state.config.env.is_production() {
        let origin = headers.get(ORIGIN).and_then(|v| v.to_str().ok());
        if origin != Some(state.config.cors_origin.as_str()) {
            return Err(Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body("Invalid origin".into())
                .unwrap_or_default());
        }
    }

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state)))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let auth = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next()).await;
    let (ticket, nonce) = match auth {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Auth { ticket, nonce }) => (ticket, nonce),
            _ => {
                let _ = socket
                    .send(Message::Text(
                        ServerMessage::Error {
                            message: "Expected auth message".into(),
                        }
                        .to_json()
                        .into(),
                    ))
                    .await;
                return;
            }
        },
        _ => {
            let _ = socket
                .send(Message::Text(
                    ServerMessage::Error {
                        message: "Authentication timeout".into(),
                    }
                    .to_json()
                    .into(),
                ))
                .await;
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Consume ticket atomically (GETDEL) and validate the bound nonce in
    // constant time. Credentials arrive in the first WS frame, not the URL,
    // so reverse-proxy access logs never see them.
    let user_id = match ws_ticket::consume_ticket(&state.redis, &ticket, &nonce).await {
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
    if ws_sender
        .send(Message::Text(auth_ok.to_json().into()))
        .await
        .is_err()
    {
        return;
    }

    // Set user online
    let _ = presence::set_online_with_ttl(&state.redis, &user_id, 300).await;
    crate::metrics::record_ws_connect();

    // Channel for outgoing messages (writer task reads from this)
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);

    // Snapshot whether the user already had another connection BEFORE this
    // one registers — multi-tab users shouldn't re-broadcast "online" every
    // time they open a new tab. The check must come before `register`.
    let was_offline_before_register = !state.user_connections.is_online(&user_id);

    // Generate a unique connection ID and register
    let conn_id = uuid::Uuid::new_v4().to_string();
    state
        .user_connections
        .register(&user_id, conn_id.clone(), out_tx.clone());

    // Presence audience: graph-scoped union of friends + server co-members.
    // Cached locally with a TTL so a block / unfriend / server-leave mid
    // session takes effect within `AUDIENCE_TTL` instead of waiting for the
    // user to reconnect — Phase 1.3 spec wants presence scoped to live graph
    // relationships, not the snapshot taken at connect time.
    let mut audience = compute_presence_audience(&state, &user_id).await;
    let mut audience_fetched_at = std::time::Instant::now();

    let online_msg = ServerMessage::Presence {
        user_id: user_id.clone(),
        status: "online".to_string(),
    }
    .to_json();

    if was_offline_before_register
        && presence::try_claim_flap_slot(&state.redis, &user_id, "online").await
    {
        for audience_id in &audience {
            if state.user_connections.is_online(audience_id) {
                state
                    .user_connections
                    .send_to_user(audience_id, online_msg.clone())
                    .await;
            }
        }
    } else if was_offline_before_register {
        crate::metrics::record_presence_flap_suppressed();
        tracing::debug!(%user_id, "suppressed online broadcast — flap slot held");
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
    // For min-interval enforcement on heartbeats.
    let mut last_heartbeat_at: Option<std::time::Instant> = None;

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
                    if presence::try_claim_flap_slot(&state.redis, &user_id, "idle").await {
                        refresh_audience_if_stale(
                            &state, &user_id,
                            &mut audience, &mut audience_fetched_at,
                        ).await;
                        let idle_msg = ServerMessage::Presence {
                            user_id: user_id.clone(),
                            status: "idle".to_string(),
                        }.to_json();
                        for audience_id in &audience {
                            state.user_connections.send_to_user(audience_id, idle_msg.clone()).await;
                        }
                    } else {
                        crate::metrics::record_presence_flap_suppressed();
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
                        // Already authenticated via the first frame; ignore repeats.
                    }
                    ClientMessage::Heartbeat => {
                        let now = std::time::Instant::now();
                        if let Some(prev) = last_heartbeat_at {
                            if now.duration_since(prev) < MIN_HEARTBEAT_INTERVAL {
                                tracing::warn!(
                                    %user_id,
                                    "heartbeat min-interval violated; dropping"
                                );
                                continue;
                            }
                        }
                        last_heartbeat_at = Some(now);
                        heartbeat_deadline = tokio::time::Instant::now() + heartbeat_timeout;
                        idle_deadline = tokio::time::Instant::now() + idle_timeout;

                        // Restore from idle if needed
                        if is_idle {
                            is_idle = false;
                            let _ =
                                presence::set_online_with_ttl(&state.redis, &user_id, 300).await;
                            if presence::try_claim_flap_slot(&state.redis, &user_id, "online").await
                            {
                                refresh_audience_if_stale(
                                    &state,
                                    &user_id,
                                    &mut audience,
                                    &mut audience_fetched_at,
                                )
                                .await;
                                let online_msg = ServerMessage::Presence {
                                    user_id: user_id.clone(),
                                    status: "online".to_string(),
                                }
                                .to_json();
                                for audience_id in &audience {
                                    state
                                        .user_connections
                                        .send_to_user(audience_id, online_msg.clone())
                                        .await;
                                }
                            } else {
                                crate::metrics::record_presence_flap_suppressed();
                            }
                        } else {
                            let _ =
                                presence::set_online_with_ttl(&state.redis, &user_id, 300).await;
                        }

                        let _ = out_tx.send(ServerMessage::HeartbeatAck.to_json()).await;
                    }
                    ClientMessage::Subscribe { channel_id, level } => {
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
                            subscriptions.insert(channel_id.clone(), SubscriptionLevel::Active);

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
                        if let Err(e) = state.collab.subscribe(&r, &user_id, out_tx.clone()).await {
                            CollabManager::send_error(&out_tx, &r, "subscribe_failed", &e).await;
                        }
                    }
                    ClientMessage::CollabUnsubscribe { post_id } => {
                        let r = ResourceRef::post(post_id);
                        collab_subscriptions.remove(&r);
                        state.collab.unsubscribe(&r, &user_id).await;
                    }
                    ClientMessage::CollabUpdate {
                        post_id,
                        update_b64,
                    } => {
                        let r = ResourceRef::post(post_id);
                        if let Err(e) = state.collab.apply_update(&r, &user_id, &update_b64).await {
                            CollabManager::send_error(&out_tx, &r, "update_failed", &e).await;
                        }
                    }
                    ClientMessage::AwarenessUpdate {
                        post_id,
                        state: aw_state,
                    } => {
                        if awareness_too_large(&aw_state) {
                            continue;
                        }
                        let r = ResourceRef::post(post_id);
                        let rate_key = whiteboard_awareness_key(&user_id, &r.id);
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: rate_key,
                                limit: 2,
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
                        if let Err(e) = state.collab.subscribe(&r, &user_id, out_tx.clone()).await {
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
                        if let Err(e) = state.collab.apply_update(&r, &user_id, &update_b64).await {
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
                        // Awareness fan-out is amplified to every peer; cap
                        // at 2/sec to bound broadcast bandwidth even with the
                        // size cap, in addition to the per-frame check above.
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: rate_key,
                                limit: 2,
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
                        // Transfer broadcasts to every viewer, so a leader
                        // ping-ponging leadership is a fan-out amplifier. Reuse
                        // the queue-op bucket — it's already keyed per user
                        // per room and gives 10/min, plenty for legitimate use.
                        if !check_watch_queue_rate(&state, &user_id, &channel_id, &out_tx).await {
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
                    ClientMessage::WatchQueueAdd {
                        channel_id,
                        video_id,
                        title,
                        duration_ms,
                        thumbnail_url,
                        nonce,
                    } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        if title.chars().count() > MAX_WATCH_TITLE_LEN {
                            let err = ServerMessage::WatchError {
                                channel_id: channel_id.clone(),
                                code: "TITLE_TOO_LONG".into(),
                                message: format!("title exceeds {MAX_WATCH_TITLE_LEN} characters"),
                            }
                            .to_json();
                            let _ = out_tx.send(err).await;
                            continue;
                        }
                        if !check_watch_queue_rate(&state, &user_id, &channel_id, &out_tx).await {
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::QueueAdd {
                                    from_user: user_id.clone(),
                                    video_id,
                                    title,
                                    duration_ms,
                                    thumbnail_url,
                                    nonce,
                                    reply_to: out_tx.clone(),
                                })
                                .await;
                        }
                    }
                    ClientMessage::WatchQueueRemove {
                        channel_id,
                        item_id,
                    } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        if !check_watch_queue_rate(&state, &user_id, &channel_id, &out_tx).await {
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::QueueRemove {
                                    from_user: user_id.clone(),
                                    item_id,
                                    reply_to: out_tx.clone(),
                                })
                                .await;
                        }
                    }
                    ClientMessage::WatchVote {
                        channel_id,
                        item_id,
                        value,
                    } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        if !check_watch_queue_rate(&state, &user_id, &channel_id, &out_tx).await {
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::Vote {
                                    from_user: user_id.clone(),
                                    item_id,
                                    value,
                                    reply_to: out_tx.clone(),
                                })
                                .await;
                        }
                    }
                    ClientMessage::WatchSkip { channel_id } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        if !check_watch_queue_rate(&state, &user_id, &channel_id, &out_tx).await {
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::Skip {
                                    from_user: user_id.clone(),
                                    reply_to: out_tx.clone(),
                                })
                                .await;
                        }
                    }
                    ClientMessage::WatchReaction { channel_id, emoji } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        // Reject empty or oversized payloads — emojis can be
                        // multi-codepoint (e.g. flag sequences) so we allow
                        // up to 32 bytes, plenty for any single emoji.
                        let trimmed = emoji.trim();
                        if trimmed.is_empty() || trimmed.len() > 32 {
                            continue;
                        }
                        let rate_key = watch_reaction_key(&user_id, &channel_id);
                        if check_rate_limit(
                            &state.redis,
                            &RateLimitConfig {
                                key_prefix: rate_key,
                                limit: 5,
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
                                        message: "Reaction rate limited".into(),
                                    }
                                    .to_json(),
                                )
                                .await;
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::Reaction {
                                    from_user: user_id.clone(),
                                    username: username.clone(),
                                    emoji: trimmed.to_string(),
                                })
                                .await;
                        }
                    }
                    ClientMessage::WatchProgress {
                        channel_id,
                        position_ms,
                    } => {
                        if !watch_subscriptions.contains(&channel_id) {
                            send_watch_not_subscribed(&out_tx, &channel_id).await;
                            continue;
                        }
                        // Defense-in-depth cap on the progress stream. The
                        // leader's client emits ~once every 5s; a hostile or
                        // bugged leader spamming faster would still be
                        // bounded by the actor's `current_recorded` one-shot
                        // gate, but rate-limiting also protects the per-op
                        // `Progress` mailbox slot from saturation.
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
                            continue;
                        }
                        if let Some(room) = state.watch_manager.get_room(&channel_id).await {
                            let _ = room
                                .send(WatchCommand::Progress {
                                    from_user: user_id.clone(),
                                    position_ms,
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

    // Cleanup — unregister first so `is_online` reflects whether any *other*
    // connection survives this disconnect (e.g. another tab).
    crate::metrics::record_ws_disconnect();
    state.user_connections.unregister(&user_id, &conn_id);

    // 30-second offline grace period. Phase 1.3 spec: don't flap to "offline"
    // on a brief network blip — the user reopening their laptop within 30s
    // should never trigger an offline → online round-trip for the whole
    // audience. We spawn a detached task that re-checks `is_online` after
    // the grace window; if any other connection arrived (or stayed) we skip
    // the offline broadcast entirely.
    if !state.user_connections.is_online(&user_id) {
        let state_for_grace = state.clone();
        let user_for_grace = user_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if state_for_grace.user_connections.is_online(&user_for_grace) {
                return;
            }
            let _ = presence::set_offline(&state_for_grace.redis, &user_for_grace).await;
            // Same flap slot as online/idle transitions — if the user just
            // emitted any presence event within the lock TTL, suppress the
            // offline echo to keep audience traffic bounded under a
            // reconnect-loop attacker.
            if !presence::try_claim_flap_slot(&state_for_grace.redis, &user_for_grace, "offline")
                .await
            {
                crate::metrics::record_presence_flap_suppressed();
                return;
            }
            // Re-resolve audience fresh: 30s have passed, the user may have
            // joined/left servers or blocked someone before disconnecting.
            // We want the offline event to honor the current graph, not
            // whatever was cached at connect time.
            let fresh_audience = compute_presence_audience(&state_for_grace, &user_for_grace).await;
            let offline_msg = ServerMessage::Presence {
                user_id: user_for_grace.clone(),
                status: "offline".to_string(),
            }
            .to_json();
            for audience_id in &fresh_audience {
                state_for_grace
                    .user_connections
                    .send_to_user(audience_id, offline_msg.clone())
                    .await;
            }
        });
    }

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
