use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use crate::repositories::watch::{PlaybackPersist, WatchRepo};
use crate::ws::protocol::ServerMessage;
use crate::ws::watch_room_manager::WatchRoomManager;
use crate::ws::watch_types::{
    PlaybackSummary, QueueItemSummary, ViewerSummary, WatchCommand,
};

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
    /// it plays). `None` when nothing is playing. Populated by later commits
    /// when the queue + playback arms land.
    current_item_id: Option<String>,
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

        // Hydrate from DB so a restarted server reloads queue + last playback
        // (queue/current_item hydration lands in a later commit alongside the
        // queue command arms; for now just ensure the row exists).
        if let Err(e) = watch_repo.ensure_room(&channel_id).await {
            tracing::warn!(channel = %channel_id, error = %e, "Failed to ensure watch_room row");
        }
        if let Ok(Some(room)) = watch_repo.find_room(&channel_id).await {
            // Pull the persisted leader so a reconnect mid-session restores
            // it; queue + current_item hydration is deferred.
            state.leader_id = room.leader.as_ref().map(|r| r.key().to_string());
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

            // Send the room state directly to the joiner so they can hydrate
            // before any subsequent broadcasts arrive.
            let snapshot = state.state_message().to_json();
            if sender.try_send(snapshot).is_err() {
                tracing::warn!(%user_id, "Failed to send watch_state to new subscriber");
            }

            let updated = state.state_message().to_json();
            let dead = state.broadcast(updated, Some(&user_id));
            state.reap_dead(dead);

            if promoted {
                tracing::info!(
                    leader = %user_id,
                    channel = %state.channel_id,
                    "Promoted first joiner to leader"
                );
                let _ = persist_playback(state, watch_repo).await;
            }
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

                if was_leader {
                    let _ = persist_playback(state, watch_repo).await;
                }
            }
        }
        WatchCommand::TransferLeader {
            from_user,
            to_user,
            reply_to,
        } => {
            if state.leader_id.as_deref() != Some(from_user.as_str()) {
                send_error(&reply_to, &state.channel_id, "not_leader",
                    "Only the current leader can transfer leadership");
                return;
            }
            if !state.subscribers.contains_key(&to_user) {
                send_error(&reply_to, &state.channel_id, "target_not_connected",
                    "Target user is not currently in the room");
                return;
            }
            state.leader_id = Some(to_user.clone());
            let msg = ServerMessage::WatchLeaderChanged {
                channel_id: state.channel_id.clone(),
                leader_id: to_user,
                reason: "transfer".into(),
            }
            .to_json();
            let dead = state.broadcast(msg, None);
            state.reap_dead(dead);
            let _ = persist_playback(state, watch_repo).await;
        }
        WatchCommand::PlaybackControl {
            from_user,
            action,
            position_ms,
            reply_to,
        } => {
            if state.leader_id.as_deref() != Some(from_user.as_str()) {
                send_error(&reply_to, &state.channel_id, "not_leader",
                    "Only the leader can control playback");
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
                send_error(&reply_to, &state.channel_id, "bad_action",
                    "action must be play, pause, or seek");
                return;
            }
            // Reject negative positions — common cause is a bad client clock.
            let clamped = position_ms.max(0);
            let server_ts = now_ms();
            state.playback.paused = new_paused;
            state.playback.position_ms = clamped;
            state.playback.server_ts = server_ts;

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
            let _ = persist_playback(state, watch_repo).await;
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

fn send_error(tx: &mpsc::Sender<String>, channel_id: &str, code: &str, message: &str) {
    let _ = tx.try_send(
        ServerMessage::WatchError {
            channel_id: channel_id.to_string(),
            code: code.to_string(),
            message: message.to_string(),
        }
        .to_json(),
    );
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
