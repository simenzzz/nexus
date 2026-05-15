use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::ws::room::{spawn_room, RoomCommand};

pub struct RoomManager {
    rooms: Arc<RwLock<HashMap<String, mpsc::Sender<RoomCommand>>>>,
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_or_create(&self, channel_id: &str) -> mpsc::Sender<RoomCommand> {
        {
            let rooms = self.rooms.read().await;
            if let Some(sender) = rooms.get(channel_id) {
                return sender.clone();
            }
        }

        let mut rooms = self.rooms.write().await;
        // Double-check after acquiring write lock
        if let Some(sender) = rooms.get(channel_id) {
            return sender.clone();
        }

        let sender = spawn_room(channel_id.to_string(), self.clone());
        rooms.insert(channel_id.to_string(), sender.clone());
        sender
    }

    pub async fn remove(&self, channel_id: &str) {
        let mut rooms = self.rooms.write().await;
        rooms.remove(channel_id);
    }

    pub async fn get_room(&self, channel_id: &str) -> Option<mpsc::Sender<RoomCommand>> {
        let rooms = self.rooms.read().await;
        rooms.get(channel_id).cloned()
    }
}

impl Clone for RoomManager {
    fn clone(&self) -> Self {
        Self {
            rooms: Arc::clone(&self.rooms),
        }
    }
}
