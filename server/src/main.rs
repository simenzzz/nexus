mod auth;
mod config;
mod error;
mod graph;
mod handlers;
mod models;
mod ws;

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use config::AppConfig;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    // TODO: pub db: surrealdb::Surreal<surrealdb::engine::remote::ws::Client>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = AppConfig::from_env()?;
    let addr = format!("{}:{}", config.server_host, config.server_port);

    let state = AppState { config };

    let app = Router::new()
        // Auth
        .route("/api/auth/register", post(handlers::users::create_user))
        .route("/api/auth/login", post(handlers::users::login))
        // Users
        .route("/api/users/{id}", get(handlers::users::get_user))
        // Servers
        .route(
            "/api/servers",
            get(handlers::servers::list_servers).post(handlers::servers::create_server),
        )
        .route("/api/servers/{id}", get(handlers::servers::get_server))
        .route(
            "/api/servers/{id}/join",
            post(handlers::servers::join_server),
        )
        // Channels
        .route(
            "/api/servers/{server_id}/channels",
            get(handlers::channels::get_channels).post(handlers::channels::create_channel),
        )
        // Messages
        .route(
            "/api/channels/{channel_id}/messages",
            get(handlers::messages::get_messages),
        )
        // WebSocket
        .route("/ws", get(ws::connection::handle_ws_upgrade))
        // Middleware
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    tracing::info!("Nexus server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
