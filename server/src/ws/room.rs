use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::ws::protocol::SubscriptionLevel;
use crate::ws::room_manager::RoomManager;

pub enum RoomCommand {
    Join {
        user_id: String,
        username: String,
        level: SubscriptionLevel,
        sender: mpsc::Sender<String>,
    },
    Leave {
        user_id: String,
    },
    UpdateLevel {
        user_id: String,
        level: SubscriptionLevel,
    },
    Broadcast {
        message: String,
        exclude_user: Option<String>,
    },
    SendToUser {
        user_id: String,
        message: String,
    },
}

struct Subscriber {
    username: String,
    level: SubscriptionLevel,
    sender: mpsc::Sender<String>,
}

pub struct Room {
    channel_id: String,
    clients: HashMap<String, Subscriber>,
    room_manager: RoomManager,
}

impl Room {
    fn new(channel_id: String, room_manager: RoomManager) -> Self {
        Self {
            channel_id,
            clients: HashMap::new(),
            room_manager,
        }
    }

    fn handle_command(&mut self, cmd: RoomCommand) {
        match cmd {
            RoomCommand::Join {
                user_id,
                username,
                level,
                sender,
            } => {
                tracing::info!(%user_id, channel = %self.channel_id, "User joined room");
                self.clients.insert(
                    user_id,
                    Subscriber {
                        username,
                        level,
                        sender,
                    },
                );
            }
            RoomCommand::Leave { user_id } => {
                tracing::info!(%user_id, channel = %self.channel_id, "User left room");
                self.clients.remove(&user_id);
            }
            RoomCommand::UpdateLevel { user_id, level } => {
                if let Some(sub) = self.clients.get_mut(&user_id) {
                    sub.level = level;
                }
            }
            RoomCommand::Broadcast {
                message,
                exclude_user,
            } => {
                let dead_clients: Vec<String> = self
                    .clients
                    .iter()
                    .filter(|(id, _)| exclude_user.as_ref() != Some(*id))
                    .filter_map(|(id, sub)| {
                        let msg = match sub.level {
                            SubscriptionLevel::Active => message.clone(),
                            SubscriptionLevel::Badge => message.clone(),
                        };
                        match sub.sender.try_send(msg) {
                            Ok(()) => None,
                            Err(mpsc::error::TrySendError::Closed(_)) => Some(id.clone()),
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                tracing::warn!(user_id = %id, channel = %self.channel_id, "Client send buffer full, dropping message");
                                None
                            }
                        }
                    })
                    .collect();

                for id in dead_clients {
                    self.clients.remove(&id);
                }
            }
            RoomCommand::SendToUser { user_id, message } => {
                if let Some(sub) = self.clients.get(&user_id) {
                    let _ = sub.sender.try_send(message);
                }
            }
        }
    }
}

const GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(30);

pub fn spawn_room(channel_id: String, room_manager: RoomManager) -> mpsc::Sender<RoomCommand> {
    let (tx, mut rx) = mpsc::channel::<RoomCommand>(256);

    tokio::spawn(async move {
        let mut room = Room::new(channel_id.clone(), room_manager);
        tracing::info!(channel = %channel_id, "Room actor started");

        while let Some(cmd) = rx.recv().await {
            room.handle_command(cmd);

            if room.clients.is_empty() {
                tracing::debug!(channel = %channel_id, "Room empty, waiting grace period");
                // Grace period: wait for someone to rejoin before shutting down
                match tokio::time::timeout(GRACE_PERIOD, rx.recv()).await {
                    Ok(Some(cmd)) => {
                        room.handle_command(cmd);
                        continue;
                    }
                    _ => {
                        tracing::info!(channel = %channel_id, "Room empty after grace period, shutting down");
                        room.room_manager.remove(&channel_id).await;
                        break;
                    }
                }
            }
        }

        tracing::info!(channel = %channel_id, "Room actor stopped");
    });

    tx
}
