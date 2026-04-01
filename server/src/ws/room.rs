use std::collections::HashMap;
use tokio::sync::mpsc;

pub enum RoomMessage {
    Join {
        user_id: String,
        sender: mpsc::Sender<String>,
    },
    Leave {
        user_id: String,
    },
    Broadcast {
        from: String,
        message: String,
    },
}

pub struct Room {
    pub channel_id: String,
    clients: HashMap<String, mpsc::Sender<String>>,
}

impl Room {
    fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            clients: HashMap::new(),
        }
    }

    fn handle_message(&mut self, msg: RoomMessage) {
        match msg {
            RoomMessage::Join { user_id, sender } => {
                tracing::info!(%user_id, channel = %self.channel_id, "User joined room");
                self.clients.insert(user_id, sender);
            }
            RoomMessage::Leave { user_id } => {
                tracing::info!(%user_id, channel = %self.channel_id, "User left room");
                self.clients.remove(&user_id);
            }
            RoomMessage::Broadcast { from, message } => {
                tracing::debug!(%from, channel = %self.channel_id, "Broadcasting message");
                let dead_clients: Vec<String> = self
                    .clients
                    .iter()
                    .filter(|(id, _)| **id != from)
                    .filter_map(|(id, sender)| {
                        if sender.try_send(message.clone()).is_err() {
                            Some(id.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                for id in dead_clients {
                    self.clients.remove(&id);
                }
            }
        }
    }
}

/// Spawn a room actor as a Tokio task. Returns a sender to communicate with the room.
pub fn spawn_room(channel_id: String) -> mpsc::Sender<RoomMessage> {
    let (tx, mut rx) = mpsc::channel::<RoomMessage>(256);

    tokio::spawn(async move {
        let mut room = Room::new(channel_id.clone());
        tracing::info!(channel = %channel_id, "Room actor started");

        while let Some(msg) = rx.recv().await {
            room.handle_message(msg);

            if room.clients.is_empty() {
                tracing::info!(channel = %channel_id, "Room is empty, shutting down");
                break;
            }
        }

        tracing::info!(channel = %channel_id, "Room actor stopped");
    });

    tx
}
