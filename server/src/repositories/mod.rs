pub mod channel;
pub mod message;
pub mod server;
pub mod social;
pub mod user;

use std::sync::Arc;

use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

#[derive(Clone)]
pub struct Repos {
    pub users: Arc<dyn user::UserRepo>,
    pub servers: Arc<dyn server::ServerRepo>,
    pub channels: Arc<dyn channel::ChannelRepo>,
    pub messages: Arc<dyn message::MessageRepo>,
    pub social: Arc<dyn social::SocialRepo>,
}

impl Repos {
    pub fn new(db: Surreal<Client>) -> Self {
        Self {
            users: Arc::new(user::SurrealUserRepo::new(db.clone())),
            servers: Arc::new(server::SurrealServerRepo::new(db.clone())),
            channels: Arc::new(channel::SurrealChannelRepo::new(db.clone())),
            messages: Arc::new(message::SurrealMessageRepo::new(db.clone())),
            social: Arc::new(social::SurrealSocialRepo::new(db)),
        }
    }
}
