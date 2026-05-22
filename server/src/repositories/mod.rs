pub mod channel;
pub mod message;
pub mod post;
pub mod recommendations;
pub mod server;
pub mod social;
pub mod user;
pub mod watch;
pub mod whiteboard;

use std::sync::Arc;

use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

#[derive(Clone)]
pub struct Repos {
    pub users: Arc<dyn user::UserRepo>,
    pub servers: Arc<dyn server::ServerRepo>,
    pub channels: Arc<dyn channel::ChannelRepo>,
    pub messages: Arc<dyn message::MessageRepo>,
    pub social: Arc<dyn social::SocialRepo>,
    pub posts: Arc<dyn post::PostRepo>,
    pub whiteboards: Arc<dyn whiteboard::WhiteboardRepo>,
    pub watch: Arc<dyn watch::WatchRepo>,
    pub recommendations: Arc<dyn recommendations::RecommendationsRepo>,
}

impl Repos {
    pub fn new(db: Surreal<Client>) -> Self {
        Self {
            users: Arc::new(user::SurrealUserRepo::new(db.clone())),
            servers: Arc::new(server::SurrealServerRepo::new(db.clone())),
            channels: Arc::new(channel::SurrealChannelRepo::new(db.clone())),
            messages: Arc::new(message::SurrealMessageRepo::new(db.clone())),
            social: Arc::new(social::SurrealSocialRepo::new(db.clone())),
            posts: Arc::new(post::SurrealPostRepo::new(db.clone())),
            whiteboards: Arc::new(whiteboard::SurrealWhiteboardRepo::new(db.clone())),
            watch: Arc::new(watch::SurrealWatchRepo::new(db.clone())),
            recommendations: Arc::new(recommendations::SurrealRecommendationsRepo::new(db)),
        }
    }
}
