mod auth;
mod config;
mod error;
mod handlers;
mod metrics;
mod middleware;
mod models;
mod repositories;
mod validation;
mod ws;

use axum::extract::FromRef;
use axum::http::{HeaderValue, Method};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{delete, get, post};
use axum::Router;
use deadpool_redis::{Config as RedisConfig, Pool, Runtime};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use config::AppConfig;
use repositories::Repos;
use ws::room_manager::RoomManager;
use ws::user_connections::UserConnectionRegistry;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: Surreal<Client>,
    pub redis: Pool,
    pub repos: Repos,
    pub room_manager: RoomManager,
    pub user_connections: UserConnectionRegistry,
    pub metrics_handle: metrics_exporter_prometheus::PrometheusHandle,
}

impl FromRef<AppState> for Repos {
    fn from_ref(state: &AppState) -> Self {
        state.repos.clone()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Structured JSON logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    // Initialize Prometheus metrics exporter
    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()?;

    let config = AppConfig::from_env()?;

    // Connect to SurrealDB
    let db = Surreal::new::<Ws>(&config.surreal_url).await?;
    db.signin(Root {
        username: &config.surreal_user,
        password: &config.surreal_pass,
    })
    .await?;
    db.use_ns(&config.surreal_ns).use_db(&config.surreal_db).await?;

    // Create Redis pool
    let redis_pool = RedisConfig::from_url(&config.redis_url)
        .create_pool(Some(Runtime::Tokio1))?;

    let repos = Repos::new(db.clone());
    let room_manager = RoomManager::new();
    let user_connections = UserConnectionRegistry::new();

    let state = AppState {
        config,
        db,
        redis: redis_pool,
        repos,
        room_manager,
        user_connections,
        metrics_handle: prometheus_handle,
    };

    let addr = format!(
        "{}:{}",
        state.config.server_host, state.config.server_port
    );

    // Health routes — no rate limiting
    let health_routes = Router::new()
        .route("/health", get(handlers::health::health))
        .route("/ready", get(handlers::health::ready))
        .route("/metrics", get(handlers::health::metrics_handler));

    // API + WS routes — rate limited
    let api_routes = Router::new()
        // Auth routes (per-IP rate limiting in handlers)
        .route("/api/auth/register", post(handlers::users::create_user))
        .route("/api/auth/login", post(handlers::users::login))
        .route("/api/auth/refresh", post(handlers::users::refresh))
        .route("/api/auth/ws-ticket", post(handlers::users::ws_ticket))
        .route("/api/auth/logout", post(handlers::users::logout))
        .route("/api/auth/me", get(handlers::users::me))
        // User routes
        .route("/api/users/{id}", get(handlers::users::get_user))
        // Friend routes
        .route("/api/friends", get(handlers::social::list_friends))
        .route("/api/friends/request", post(handlers::social::send_friend_request))
        .route("/api/friends/accept", post(handlers::social::accept_friend_request))
        .route("/api/friends/pending", get(handlers::social::list_pending_incoming))
        .route("/api/friends/suggestions", get(handlers::social::get_friend_suggestions))
        .route("/api/friends/mutual/{user_id}", get(handlers::social::get_mutual_friends))
        .route("/api/friends/{user_id}", delete(handlers::social::remove_friend))
        // Follow routes
        .route("/api/users/{user_id}/follow", post(handlers::social::follow_user))
        .route("/api/users/{user_id}/follow", delete(handlers::social::unfollow_user))
        // Block routes
        .route("/api/users/{user_id}/block", post(handlers::social::block_user))
        .route("/api/users/{user_id}/block", delete(handlers::social::unblock_user))
        // Server routes
        .route(
            "/api/servers",
            get(handlers::servers::list_servers).post(handlers::servers::create_server),
        )
        .route("/api/servers/{id}", get(handlers::servers::get_server))
        .route(
            "/api/servers/{id}/join",
            post(handlers::servers::join_server),
        )
        // Channel routes
        .route(
            "/api/servers/{server_id}/channels",
            get(handlers::channels::get_channels).post(handlers::channels::create_channel),
        )
        // Message routes
        .route(
            "/api/channels/{channel_id}/messages",
            get(handlers::messages::get_messages),
        )
        // Discovery routes
        .route("/api/discover/servers", get(handlers::discovery::discover_servers))
        .layer(from_fn(
            middleware::request_id::request_id_middleware,
        ))
        .layer(from_fn_with_state(
            state.clone(),
            middleware::api_rate_limit::api_rate_limit_middleware,
        ))
        // WebSocket
        .route("/ws", get(ws::connection::handle_ws_upgrade));

    let cors_origin: HeaderValue = state
        .config
        .cors_origin
        .parse()
        .expect("CORS_ORIGIN must be a valid header value");

    let app = Router::new()
        .merge(health_routes)
        .merge(api_routes)
        .layer(
            CorsLayer::new()
                .allow_origin(cors_origin)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE, COOKIE])
                .allow_credentials(true),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    tracing::info!("Nexus server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}
