use std::collections::HashMap;

use crate::models::user::UserStatus;

pub struct PresenceTracker {
    statuses: HashMap<String, UserStatus>,
}

impl PresenceTracker {
    pub fn new() -> Self {
        Self {
            statuses: HashMap::new(),
        }
    }

    pub fn set_online(&mut self, user_id: String) {
        self.statuses.insert(user_id, UserStatus::Online);
    }

    pub fn set_offline(&mut self, user_id: String) {
        self.statuses.insert(user_id, UserStatus::Offline);
    }

    pub fn get_status(&self, user_id: &str) -> UserStatus {
        self.statuses
            .get(user_id)
            .cloned()
            .unwrap_or(UserStatus::Offline)
    }

    pub fn is_online(&self, user_id: &str) -> bool {
        matches!(self.statuses.get(user_id), Some(UserStatus::Online))
    }
}
