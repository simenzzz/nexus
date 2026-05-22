use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::repositories::watch::{PlaybackPersist, WatchRepo};
use crate::ws::protocol::ServerMessage;
use crate::ws::watch_room_helpers::{
    is_valid_reaction_emoji, is_valid_youtube_id, now_ms, send_error, sort_queue,
};
use crate::ws::watch_room_manager::WatchRoomManager;
use crate::ws::watch_types::{PlaybackSummary, QueueItemSummary, ViewerSummary, WatchCommand};

struct Subscriber {
    username: String,
    sender: mpsc::Sender<String>,
    /// Monotonic join time, used by leader-transfer to pick the
    /// longest-connected member when the current leader drops.
    joined_at: tokio::time::Instant,
}

struct WatchRoomState {
    channel_id: String,
    subscribers: HashMap<String, Subscriber>,
    leader_id: Option<String>,
    playback: PlaybackSummary,
    queue: Vec<QueueItemSummary>,
    /// The id of the queue item currently playing (kept out of `queue` while
    /// it plays). `None` when nothing is playing.
    current_item_id: Option<String>,
    /// Duration of the current item, used for the >=90% completion gate.
    /// 0 means "unknown" — completion detection is skipped in that case.
    current_duration_ms: i64,
    /// Marks whether `record_watched` has fired for the current item this
    /// session. Reset on advance. Prevents duplicate edge writes from a
    /// chatty progress stream.
    current_recorded: bool,
}

impl WatchRoomState {
    fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            subscribers: HashMap::new(),
            leader_id: None,
            playback: PlaybackSummary {
                video_id: None,
                position_ms: 0,
                paused: true,
                server_ts: now_ms(),
                rate: 1.0,
            },
            queue: Vec::new(),
            current_item_id: None,
            current_duration_ms: 0,
            current_recorded: false,
        }
    }

    fn viewers(&self) -> Vec<ViewerSummary> {
        self.subscribers
            .iter()
            .map(|(id, sub)| ViewerSummary {
                user_id: id.clone(),
                username: sub.username.clone(),
                is_leader: self.leader_id.as_deref() == Some(id.as_str()),
            })
            .collect()
    }

    fn state_message(&self) -> ServerMessage {
        ServerMessage::WatchState {
            channel_id: self.channel_id.clone(),
            leader_id: self.leader_id.clone(),
            playback: serde_json::to_value(&self.playback).unwrap_or(serde_json::Value::Null),
            queue: serde_json::to_value(&self.queue).unwrap_or(serde_json::Value::Array(vec![])),
            viewers: serde_json::to_value(self.viewers())
                .unwrap_or(serde_json::Value::Array(vec![])),
        }
    }

    /// Fire-and-forget fanout to every subscriber, optionally skipping one.
    /// Returns the list of subscribers whose channels are closed so the
    /// caller can reap them.
    fn broadcast(&self, payload: String, exclude_user: Option<&str>) -> Vec<String> {
        let mut dead = Vec::new();
        for (id, sub) in &self.subscribers {
            if exclude_user == Some(id.as_str()) {
                continue;
            }
            match sub.sender.try_send(payload.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => dead.push(id.clone()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(
                        user_id = %id,
                        channel = %self.channel_id,
                        "watch room subscriber buffer full, dropping message"
                    );
                }
            }
        }
        dead
    }

    fn reap_dead(&mut self, dead: Vec<String>) {
        for id in dead {
            self.subscribers.remove(&id);
        }
    }
}

const GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(30);
const SYNC_PULSE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub fn spawn_watch_room(
    channel_id: String,
    manager: WatchRoomManager,
    watch_repo: Arc<dyn WatchRepo>,
) -> mpsc::Sender<WatchCommand> {
    let (tx, mut rx) = mpsc::channel::<WatchCommand>(256);

    tokio::spawn(async move {
        let mut state = WatchRoomState::new(channel_id.clone());
        tracing::info!(channel = %channel_id, "Watch room actor started");

        // Hydrate from DB so a restarted server reloads queue + last playback.
        // Errors are surfaced explicitly (rather than swallowed via `if let
        // Ok`) so a transient DB blip doesn't silently strand the actor with
        // an empty queue while disk still has rows.
        if let Err(e) = watch_repo.ensure_room(&channel_id).await {
            tracing::warn!(channel = %channel_id, error = %e, "Failed to ensure watch_room row");
        }
        match watch_repo.list_queue(&channel_id).await {
            Ok(items) => {
                state.queue = items.into_iter().map(QueueItemSummary::from).collect();
            }
            Err(e) => {
                tracing::warn!(
                    channel = %channel_id,
                    error = %e,
                    "Failed to hydrate watch queue from DB; starting empty"
                );
            }
        }
        match watch_repo.find_room(&channel_id).await {
            Ok(Some(room)) => {
                state.leader_id = room.leader.as_ref().map(|r| r.key().to_string());
                if let Some(current) = room.current_item {
                    let current_id = current.key().to_string();
                    // Pull the current item out of the queue if it's still there.
                    if let Some(pos) = state.queue.iter().position(|q| q.id == current_id) {
                        let item = state.queue.remove(pos);
                        state.playback.video_id = Some(item.video_id);
                        state.current_duration_ms = item.duration_ms;
                        state.playback.position_ms = room.playback_position_ms;
                        state.playback.paused = true; // Restart from paused — leader resumes.
                        state.playback.server_ts = now_ms();
                        state.current_item_id = Some(current_id);
                    }
                }
            }
            Ok(None) => {
                // ensure_room above should have created it; missing here is
                // benign — first-write path will populate.
            }
            Err(e) => {
                tracing::warn!(
                    channel = %channel_id,
                    error = %e,
                    "Failed to hydrate watch_room metadata from DB"
                );
            }
        }

        let mut pulse = tokio::time::interval(SYNC_PULSE_INTERVAL);
        // The actor doesn't care about catching up missed ticks — only the
        // current authoritative position matters.
        pulse.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if state.subscribers.is_empty() {
                // Grace-period shutdown: skip the pulse interval entirely and
                // wait up to 30s for a rejoin. Persist a final snapshot before
                // evicting so the room survives a restart cleanly.
                match tokio::time::timeout(GRACE_PERIOD, rx.recv()).await {
                    Ok(Some(cmd)) => handle_command(&mut state, cmd, &watch_repo).await,
                    Ok(None) => break,
                    Err(_) => {
                        let _ = persist_playback(&state, &watch_repo).await;
                        tracing::info!(channel = %channel_id, "Watch room evicted after grace period");
                        manager.remove(&channel_id).await;
                        break;
                    }
                }
                continue;
            }

            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => handle_command(&mut state, cmd, &watch_repo).await,
                    None => break,
                },
                _ = pulse.tick() => emit_sync_pulse(&mut state),
            }
        }

        tracing::info!(channel = %channel_id, "Watch room actor stopped");
    });

    tx
}

/// Periodic authoritative playback heartbeat. Only emitted while at least one
/// subscriber is present and the leader has set a video to play; pause/no-video
/// states are quiescent on the wire. No-op at P4.2 since playback lands later.
fn emit_sync_pulse(state: &mut WatchRoomState) {
    if state.playback.paused || state.playback.video_id.is_none() {
        return;
    }
    let now = now_ms();
    let elapsed_ms = now.saturating_sub(state.playback.server_ts) as i64;
    let projected = state.playback.position_ms + (elapsed_ms as f64 * state.playback.rate) as i64;

    let msg = ServerMessage::WatchSyncPulse {
        channel_id: state.channel_id.clone(),
        position_ms: projected,
        server_ts: now,
        paused: false,
    }
    .to_json();
    let dead = state.broadcast(msg, None);
    state.reap_dead(dead);
}

async fn persist_playback(
    state: &WatchRoomState,
    watch_repo: &Arc<dyn WatchRepo>,
) -> Result<(), crate::error::AppError> {
    watch_repo
        .save_playback(
            &state.channel_id,
            state.leader_id.clone(),
            PlaybackPersist {
                current_item_id: state.current_item_id.clone(),
                position_ms: state.playback.position_ms,
                paused: state.playback.paused,
            },
        )
        .await
}

async fn handle_command(
    state: &mut WatchRoomState,
    cmd: WatchCommand,
    watch_repo: &Arc<dyn WatchRepo>,
) {
    match cmd {
        WatchCommand::Join {
            user_id,
            username,
            sender,
        } => {
            tracing::info!(%user_id, channel = %state.channel_id, "User joined watch room");
            state.subscribers.insert(
                user_id.clone(),
                Subscriber {
                    username,
                    sender: sender.clone(),
                    joined_at: tokio::time::Instant::now(),
                },
            );

            // First-joiner gets leader. Subsequent joins do not change leader
            // — leadership transfers only on explicit request or disconnect.
            let promoted = state.leader_id.is_none();
            if promoted {
                state.leader_id = Some(user_id.clone());
            }

            // Best-effort persistence. Join has no reply channel; if this
            // fails, the in-memory actor remains canonical until the next save.
            if promoted {
                tracing::info!(
                    leader = %user_id,
                    channel = %state.channel_id,
                    "Promoted first joiner to leader"
                );
                if let Err(e) = persist_playback(state, watch_repo).await {
                    tracing::warn!(
                        channel = %state.channel_id,
                        error = %e,
                        "failed to persist leader promotion on join"
                    );
                    state.leader_id = None;
                }
            }

            // Send the room state directly to the joiner so they can hydrate
            // before any subsequent broadcasts arrive.
            let snapshot = state.state_message().to_json();
            if sender.try_send(snapshot).is_err() {
                tracing::warn!(%user_id, "Failed to send watch_state to new subscriber");
            }

            let updated = state.state_message().to_json();
            let dead = state.broadcast(updated, Some(&user_id));
            state.reap_dead(dead);
        }
        WatchCommand::Leave { user_id } => {
            if state.subscribers.remove(&user_id).is_some() {
                tracing::info!(%user_id, channel = %state.channel_id, "User left watch room");

                let was_leader = state.leader_id.as_deref() == Some(user_id.as_str());
                let leader_change = if was_leader {
                    // Promote longest-connected remaining member by earliest
                    // joined_at. Empty room → leader becomes None (next join
                    // gets the seat).
                    let next = state
                        .subscribers
                        .iter()
                        .min_by_key(|(_, sub)| sub.joined_at)
                        .map(|(id, _)| id.clone());
                    state.leader_id = next.clone();
                    next
                } else {
                    None
                };

                // Best-effort persistence. Disconnect has no reply channel;
                // the in-memory actor remains canonical until the next save.
                if was_leader {
                    if let Err(e) = persist_playback(state, watch_repo).await {
                        tracing::warn!(
                            channel = %state.channel_id,
                            error = %e,
                            "failed to persist leader change on leave"
                        );
                    }
                }

                if !state.subscribers.is_empty() {
                    if let Some(ref new_leader) = leader_change {
                        let msg = ServerMessage::WatchLeaderChanged {
                            channel_id: state.channel_id.clone(),
                            leader_id: new_leader.clone(),
                            reason: "disconnect".into(),
                        }
                        .to_json();
                        let dead = state.broadcast(msg, None);
                        state.reap_dead(dead);
                    }
                    let updated = state.state_message().to_json();
                    let dead = state.broadcast(updated, None);
                    state.reap_dead(dead);
                }
            }
        }
        WatchCommand::TransferLeader {
            from_user,
            to_user,
            reply_to,
        } => {
            if state.leader_id.as_deref() != Some(from_user.as_str()) {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "not_leader",
                    "Only the current leader can transfer leadership",
                );
                return;
            }
            if !state.subscribers.contains_key(&to_user) {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "target_not_connected",
                    "Target user is not currently in the room",
                );
                return;
            }
            let old_leader = state.leader_id.clone();
            state.leader_id = Some(to_user.clone());
            // Persist before broadcasting so a crash mid-transfer doesn't
            // leave the DB pointing at the old leader while clients act on
            // the new one. Persistence failure is logged but non-fatal.
            if let Err(e) = persist_playback(state, watch_repo).await {
                tracing::warn!(
                    channel = %state.channel_id,
                    error = %e,
                    "failed to persist leader transfer"
                );
                state.leader_id = old_leader;
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "persist_failed",
                    "Leadership transfer could not be saved",
                );
                return;
            }
            let msg = ServerMessage::WatchLeaderChanged {
                channel_id: state.channel_id.clone(),
                leader_id: to_user,
                reason: "transfer".into(),
            }
            .to_json();
            let dead = state.broadcast(msg, None);
            state.reap_dead(dead);
        }
        WatchCommand::PlaybackControl {
            from_user,
            action,
            position_ms,
            reply_to,
        } => {
            if state.leader_id.as_deref() != Some(from_user.as_str()) {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "not_leader",
                    "Only the leader can control playback",
                );
                return;
            }
            // Validate action whitelist defensively; the protocol carries a
            // free-form string so junk values can reach us.
            let (new_paused, valid) = match action.as_str() {
                "play" => (false, true),
                "pause" => (true, true),
                "seek" => (state.playback.paused, true),
                _ => (state.playback.paused, false),
            };
            if !valid {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "bad_action",
                    "action must be play, pause, or seek",
                );
                return;
            }
            // Reject negative positions — common cause is a bad client clock.
            let clamped = position_ms.max(0);
            let server_ts = now_ms();
            let old_playback = state.playback.clone();
            state.playback.paused = new_paused;
            state.playback.position_ms = clamped;
            state.playback.server_ts = server_ts;

            // Persist before broadcast so the DB matches what followers see.
            if let Err(e) = persist_playback(state, watch_repo).await {
                tracing::warn!(
                    channel = %state.channel_id,
                    error = %e,
                    "failed to persist playback transition"
                );
                state.playback = old_playback;
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "persist_failed",
                    "Playback transition could not be saved",
                );
                return;
            }

            let msg = ServerMessage::WatchPlayback {
                channel_id: state.channel_id.clone(),
                action,
                position_ms: clamped,
                server_ts,
                by_user: from_user,
            }
            .to_json();
            let dead = state.broadcast(msg, None);
            state.reap_dead(dead);
        }
        WatchCommand::QueueAdd {
            from_user,
            video_id,
            title,
            duration_ms,
            thumbnail_url,
            nonce,
            reply_to,
        } => {
            // Validate the YouTube video id shape — 11 chars, URL-safe alphabet.
            // Catches accidental URL-pastes and hostile payloads.
            if !is_valid_youtube_id(&video_id) {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "bad_video_id",
                    "video_id must be an 11-character YouTube id",
                );
                return;
            }
            // Defense-in-depth: the connection handler already rejects
            // oversize titles. If we somehow received one here, log loudly
            // and truncate rather than panic — the boundary check was
            // bypassed.
            let title = if title.chars().count() > 200 {
                tracing::warn!(
                    user = %from_user,
                    channel = %state.channel_id,
                    "oversize watch title hit the room actor; boundary check missed"
                );
                title.chars().take(200).collect::<String>()
            } else {
                title
            };
            let item = match watch_repo
                .add_queue_item(
                    &state.channel_id,
                    &from_user,
                    video_id,
                    title,
                    duration_ms.max(0),
                    thumbnail_url,
                )
                .await
            {
                Ok(item) => item,
                Err(e) => {
                    tracing::warn!(channel = %state.channel_id, error = %e, "queue add failed");
                    send_error(
                        &reply_to,
                        &state.channel_id,
                        "internal",
                        "Failed to add item",
                    );
                    return;
                }
            };
            let summary = QueueItemSummary::from(item);
            let item_id = summary.id.clone();

            // If nothing is playing, immediately promote this new item to
            // current — better UX than asking the leader to also click play.
            let advance = state.current_item_id.is_none() && state.playback.video_id.is_none();
            if advance {
                promote_to_current(state, summary);
                let _ = persist_playback(state, watch_repo).await;
            } else {
                state.queue.push(summary);
                sort_queue(&mut state.queue);
            }

            let ack_sent = reply_to
                .try_send(
                    ServerMessage::WatchQueueAck {
                        channel_id: state.channel_id.clone(),
                        nonce,
                        item_id,
                    }
                    .to_json(),
                )
                .is_ok();

            if advance {
                broadcast_advance(state);
            } else {
                broadcast_queue_update(state);
            }

            // If the per-client ack channel was closed or full, the client
            // never sees the targeted ack. The room-wide broadcast above
            // (queue_update or advance) lets the optimistic entry converge
            // via reconciliation rather than leaving the client spinning.
            if !ack_sent {
                tracing::warn!(
                    channel = %state.channel_id,
                    "WatchQueueAck send failed; relying on broadcast to reconcile"
                );
            }
        }
        WatchCommand::QueueRemove {
            from_user,
            item_id,
            reply_to,
        } => {
            let Some(idx) = state.queue.iter().position(|q| q.id == item_id) else {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "not_found",
                    "Queue item not found",
                );
                return;
            };
            let is_adder = state.queue[idx].added_by == from_user;
            let is_leader = state.leader_id.as_deref() == Some(from_user.as_str());
            if !is_adder && !is_leader {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "forbidden",
                    "Only the adder or the leader can remove this item",
                );
                return;
            }
            if let Err(e) = watch_repo.remove_queue_item(&item_id).await {
                tracing::warn!(channel = %state.channel_id, error = %e, "queue remove failed");
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "internal",
                    "Failed to remove item",
                );
                return;
            }
            state.queue.remove(idx);
            broadcast_queue_update(state);
        }
        WatchCommand::Vote {
            from_user,
            item_id,
            value,
            reply_to,
        } => {
            if !(-1..=1).contains(&value) {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "bad_value",
                    "vote value must be -1, 0, or 1",
                );
                return;
            }
            if !state.queue.iter().any(|q| q.id == item_id) {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "not_found",
                    "Queue item not found",
                );
                return;
            }
            let new_score = match watch_repo.set_vote(&from_user, &item_id, value).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(channel = %state.channel_id, error = %e, "vote failed");
                    send_error(
                        &reply_to,
                        &state.channel_id,
                        "internal",
                        "Failed to record vote",
                    );
                    return;
                }
            };
            // Update in-memory mirror so the broadcast reflects new ordering
            // without round-tripping list_queue.
            if let Some(item) = state.queue.iter_mut().find(|q| q.id == item_id) {
                item.score = new_score;
            }
            sort_queue(&mut state.queue);
            broadcast_queue_update(state);
        }
        WatchCommand::Skip {
            from_user,
            reply_to,
        } => {
            if state.leader_id.as_deref() != Some(from_user.as_str()) {
                send_error(
                    &reply_to,
                    &state.channel_id,
                    "not_leader",
                    "Only the leader can skip",
                );
                return;
            }
            advance_queue(state, watch_repo).await;
            broadcast_advance(state);
        }
        WatchCommand::Progress {
            from_user,
            position_ms,
        } => {
            // Leader-only — followers can't drive completion detection
            // because their playback may lag arbitrarily. Log at debug so a
            // stuck-follower or buggy client is diagnosable without giving
            // every dropped message an error-path round trip (Progress has no
            // reply channel in the protocol).
            if state.leader_id.as_deref() != Some(from_user.as_str()) {
                tracing::debug!(
                    user_id = %from_user,
                    channel = %state.channel_id,
                    "dropping watch progress from non-leader"
                );
                return;
            }
            let video_id = match state.playback.video_id.clone() {
                Some(v) => v,
                None => return,
            };
            let clamped = position_ms.max(0);
            // Keep our authoritative position fresh so a follower joining
            // between sync pulses gets accurate hydration.
            state.playback.position_ms = clamped;
            state.playback.server_ts = now_ms();

            // Completion gate: >=90% of a known duration, recorded once per
            // current item.
            if !state.current_recorded
                && state.current_duration_ms > 0
                && clamped * 10 >= state.current_duration_ms * 9
            {
                state.current_recorded = true;
                let completion = (clamped as f64 / state.current_duration_ms as f64).min(1.0);
                // Record a `watched` edge for every viewer currently in the
                // room. A single detached task walks the viewer list
                // sequentially so a large room can't fan out into N
                // concurrent DB writes per video completion (saturating the
                // Surreal connection pool). Errors are logged but not
                // surfaced — this is best-effort engagement data.
                let viewers: Vec<String> = state.subscribers.keys().cloned().collect();
                let repo = watch_repo.clone();
                let vid = video_id.clone();
                tokio::spawn(async move {
                    for user in viewers {
                        if let Err(e) = repo.record_watched(&user, &vid, completion).await {
                            tracing::warn!(
                                user_id = %user,
                                video_id = %vid,
                                error = %e,
                                "failed to record watched edge"
                            );
                        }
                    }
                });
            }

            // Auto-advance: leader reports position past the end. Saves the
            // leader from clicking Skip when a video naturally ends.
            if state.current_duration_ms > 0 && clamped >= state.current_duration_ms {
                advance_queue(state, watch_repo).await;
                broadcast_advance(state);
            }
        }
        WatchCommand::Reaction {
            from_user,
            username,
            emoji,
        } => {
            // Reactions echo to everyone including the sender so all clients
            // render identically. Skip silently if the user somehow isn't a
            // subscriber — connection.rs already enforces this, but defense
            // in depth keeps us from leaking spurious broadcasts.
            if !state.subscribers.contains_key(&from_user) {
                return;
            }
            // Defense-in-depth: the WS boundary already trims and caps at
            // 32 bytes, but the actor also rejects anything outside a safe
            // emoji shape so a future code path that bypasses connection.rs
            // can't fan out junk to every viewer.
            if !is_valid_reaction_emoji(&emoji) {
                tracing::debug!(
                    user_id = %from_user,
                    channel = %state.channel_id,
                    "dropping invalid reaction emoji"
                );
                return;
            }
            let msg = ServerMessage::WatchReaction {
                channel_id: state.channel_id.clone(),
                user_id: from_user,
                username,
                emoji,
                ts: now_ms(),
            }
            .to_json();
            let dead = state.broadcast(msg, None);
            state.reap_dead(dead);
        }
        WatchCommand::Broadcast {
            message,
            exclude_user,
        } => {
            let dead = state.broadcast(message, exclude_user.as_deref());
            state.reap_dead(dead);
        }
    }
}

fn promote_to_current(state: &mut WatchRoomState, item: QueueItemSummary) {
    state.playback.video_id = Some(item.video_id.clone());
    state.playback.position_ms = 0;
    state.playback.paused = false;
    state.playback.server_ts = now_ms();
    state.current_item_id = Some(item.id);
    state.current_duration_ms = item.duration_ms;
    state.current_recorded = false;
}

/// Move the head of the queue (highest score, oldest tie-breaker) into
/// `current_item_id` and delete the previous current item from disk if any.
async fn advance_queue(state: &mut WatchRoomState, watch_repo: &Arc<dyn WatchRepo>) {
    if let Some(prev) = state.current_item_id.take() {
        if let Err(e) = watch_repo.remove_queue_item(&prev).await {
            tracing::warn!(channel = %state.channel_id, error = %e, "failed to delete advanced item");
        }
    }
    sort_queue(&mut state.queue);
    if state.queue.is_empty() {
        state.playback.video_id = None;
        state.playback.position_ms = 0;
        state.playback.paused = true;
        state.playback.server_ts = now_ms();
        state.current_item_id = None;
        state.current_duration_ms = 0;
        state.current_recorded = false;
    } else {
        let next = state.queue.remove(0);
        promote_to_current(state, next);
    }
    let _ = persist_playback(state, watch_repo).await;
}

fn broadcast_queue_update(state: &mut WatchRoomState) {
    let msg = ServerMessage::WatchQueueUpdate {
        channel_id: state.channel_id.clone(),
        queue: serde_json::to_value(&state.queue).unwrap_or(serde_json::Value::Array(vec![])),
    }
    .to_json();
    let dead = state.broadcast(msg, None);
    state.reap_dead(dead);
}

fn broadcast_advance(state: &mut WatchRoomState) {
    let msg = ServerMessage::WatchAdvance {
        channel_id: state.channel_id.clone(),
        playback: serde_json::to_value(&state.playback).unwrap_or(serde_json::Value::Null),
        queue: serde_json::to_value(&state.queue).unwrap_or(serde_json::Value::Array(vec![])),
    }
    .to_json();
    let dead = state.broadcast(msg, None);
    state.reap_dead(dead);
}
