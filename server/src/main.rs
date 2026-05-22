mod auth;
mod collab;
mod config;
mod error;
mod handlers;
mod metrics;
mod middleware;
mod models;
mod repositories;
mod validation;
mod ws;

use axum::extract::{DefaultBodyLimit, FromRef, MatchedPath};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::middleware::{from_fn, from_fn_with_state};
use axum::routing::{delete, get, post};
use axum::Router;
use deadpool_redis::{Config as RedisConfig, Pool, Runtime};
use std::time::Duration;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use collab::post_store::PostStore;
use collab::resource::{ResourceKind, ResourceStore};
use collab::whiteboard_store::WhiteboardStore;
use collab::CollabManager;
use config::AppConfig;
use repositories::Repos;
use std::collections::HashMap;
use std::sync::Arc;
use ws::room_manager::RoomManager;
use ws::user_connections::UserConnectionRegistry;
use ws::watch_room_manager::WatchRoomManager;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: Surreal<Client>,
    pub redis: Pool,
    pub repos: Repos,
    pub room_manager: RoomManager,
    pub watch_manager: WatchRoomManager,
    pub user_connections: UserConnectionRegistry,
    pub collab: CollabManager,
    pub metrics_handle: metrics_exporter_prometheus::PrometheusHandle,
}

impl FromRef<AppState> for Repos {
    fn from_ref(state: &AppState) -> Self {
        state.repos.clone()
    }
}

impl FromRef<AppState> for CollabManager {
    fn from_ref(state: &AppState) -> Self {
        state.collab.clone()
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
    let prometheus_handle =
        metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder()?;

    // Load .env if present (no-op in production where env is provided).
    dotenvy::dotenv().ok();
    let config = AppConfig::from_env()?;

    tracing::info!(
        env = ?config.env,
        ns = %config.surreal_ns,
        db = %config.surreal_db,
        "Connecting to SurrealDB"
    );

    // Connect to SurrealDB
    let db = Surreal::new::<Ws>(&config.surreal_url).await?;
    db.signin(Root {
        username: &config.surreal_user,
        password: &config.surreal_pass,
    })
    .await?;
    db.use_ns(&config.surreal_ns)
        .use_db(&config.surreal_db)
        .await?;

    // Create Redis pool
    let redis_pool = RedisConfig::from_url(&config.redis_url).create_pool(Some(Runtime::Tokio1))?;

    let repos = Repos::new(db.clone());
    let room_manager = RoomManager::new();
    let watch_manager = WatchRoomManager::new(repos.watch.clone());
    let user_connections = UserConnectionRegistry::new();

    // Wire one ResourceStore per CRDT-backed resource kind. The manager
    // routes incoming WS messages by ResourceRef::kind into the matching
    // store for authorization + persistence.
    let mut stores: HashMap<ResourceKind, Arc<dyn ResourceStore>> = HashMap::new();
    stores.insert(
        ResourceKind::Post,
        Arc::new(PostStore::new(repos.posts.clone())),
    );
    stores.insert(
        ResourceKind::Whiteboard,
        Arc::new(WhiteboardStore::new(
            repos.whiteboards.clone(),
            repos.channels.clone(),
            repos.servers.clone(),
        )),
    );
    let collab = CollabManager::with_stores(stores, collab::SWEEP_INTERVAL, collab::IDLE_TTL);

    let state = AppState {
        config,
        db,
        redis: redis_pool,
        repos,
        room_manager,
        watch_manager,
        user_connections,
        collab,
        metrics_handle: prometheus_handle,
    };

    let addr = format!("{}:{}", state.config.server_host, state.config.server_port);

    // Health routes — no rate limiting
    let health_routes = Router::new()
        .route("/health", get(handlers::health::health))
        .route("/ready", get(handlers::health::ready))
        .route("/metrics", get(handlers::health::metrics_handler));

    // Subroutes that require double-submit CSRF protection (state-changing,
    // cookie-authenticated). Login/register can't require CSRF (no cookie
    // yet); ws-ticket and me are Bearer-authenticated and don't depend on
    // the refresh cookie.
    let csrf_protected_auth = Router::new()
        .route("/api/auth/refresh", post(handlers::users::refresh))
        .route("/api/auth/logout", post(handlers::users::logout))
        .layer(from_fn_with_state(
            state.clone(),
            middleware::csrf::csrf_middleware,
        ));

    // API + WS routes — rate limited
    let api_routes = Router::new()
        // Auth routes (per-IP rate limiting in handlers)
        .route("/api/auth/register", post(handlers::users::create_user))
        .route("/api/auth/login", post(handlers::users::login))
        .route("/api/auth/ws-ticket", post(handlers::users::ws_ticket))
        .route("/api/auth/me", get(handlers::users::me))
        .merge(csrf_protected_auth)
        // User routes
        .route("/api/users/{id}", get(handlers::users::get_user))
        // Friend routes
        .route("/api/friends", get(handlers::social::list_friends))
        .route(
            "/api/friends/request",
            post(handlers::social::send_friend_request),
        )
        .route(
            "/api/friends/accept",
            post(handlers::social::accept_friend_request),
        )
        .route(
            "/api/friends/pending",
            get(handlers::social::list_pending_incoming),
        )
        .route(
            "/api/friends/suggestions",
            get(handlers::social::get_friend_suggestions),
        )
        .route(
            "/api/friends/mutual/{user_id}",
            get(handlers::social::get_mutual_friends),
        )
        .route(
            "/api/friends/{user_id}",
            delete(handlers::social::remove_friend),
        )
        // Follow routes
        .route(
            "/api/users/{user_id}/follow",
            post(handlers::social::follow_user),
        )
        .route(
            "/api/users/{user_id}/follow",
            delete(handlers::social::unfollow_user),
        )
        // Block routes
        .route(
            "/api/users/{user_id}/block",
            post(handlers::social::block_user),
        )
        .route(
            "/api/users/{user_id}/block",
            delete(handlers::social::unblock_user),
        )
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
        .route(
            "/api/discover/servers",
            get(handlers::discovery::discover_servers),
        )
        // Post routes (Phase 2 — collaborative drafts)
        .route(
            "/api/posts",
            get(handlers::posts::list_published).post(handlers::posts::create_draft),
        )
        .route("/api/posts/{post_id}", get(handlers::posts::get_post))
        .route(
            "/api/posts/{post_id}/publish",
            post(handlers::posts::publish_post),
        )
        .route(
            "/api/posts/{post_id}/invites",
            post(handlers::posts::invite_collaborator),
        )
        // Whiteboard routes (Phase 3 — shared canvas)
        .route(
            "/api/channels/{channel_id}/whiteboard",
            get(handlers::whiteboards::get_whiteboard),
        )
        .route(
            "/api/channels/{channel_id}/whiteboard/checkpoints",
            get(handlers::whiteboards::list_checkpoints)
                .post(handlers::whiteboards::create_checkpoint),
        )
        .route(
            "/api/channels/{channel_id}/whiteboard/checkpoints/{checkpoint_id}/restore",
            post(handlers::whiteboards::restore_checkpoint),
        )
        // Watch-together routes (Phase 4 — synchronized media + recs)
        .route(
            "/api/channels/{channel_id}/watch/recommendations",
            get(handlers::watch::get_recommendations),
        )
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

    const X_CSRF_TOKEN: HeaderName = HeaderName::from_static("x-csrf-token");

    // Layer order (outermost → innermost). Each `.layer()` wraps everything
    // before it; the LAST call is the OUTERMOST middleware on the request.
    //   request_id   ─── must run first so subsequent layers/handlers can
    //                    log the correlation id
    //   TraceLayer   ─── reads the request_id extension, opens a span
    //   security_headers — wraps CORS so even preflight responses are hardened
    //   CORS         ─── short-circuits OPTIONS preflight
    //   DefaultBodyLimit — innermost: caps body before any handler reads
    let app = Router::new()
        .merge(health_routes)
        .merge(api_routes)
        // Innermost first.
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(
            CorsLayer::new()
                .allow_origin(cors_origin)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE, COOKIE, X_CSRF_TOKEN])
                .allow_credentials(true)
                .max_age(Duration::from_secs(3600)),
        )
        .layer(from_fn(
            middleware::security_headers::security_headers_middleware,
        ))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
                let path = request
                    .extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str)
                    .unwrap_or_else(|| request.uri().path());
                tracing::info_span!(
                    "request",
                    method = %request.method(),
                    path = %path,
                    version = ?request.version(),
                )
            }),
        )
        // Outermost: request_id stamps every request and response.
        .layer(from_fn(middleware::request_id::request_id_middleware))
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
