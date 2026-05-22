use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response};
use axum::middleware::Next;

use crate::auth::jwt::validate_access_token;
use crate::middleware::rate_limit::{api_general_key, check_rate_limit, RateLimitConfig};
use crate::AppState;

pub async fn api_rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response<Body> {
    // Extract user_id from Bearer token
    let user_id = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|token| validate_access_token(token, &state.config.jwt_secret).ok())
        .map(|claims| claims.sub);

    let rate_key = match &user_id {
        Some(uid) => api_general_key(uid),
        None => {
            // Fall back to IP-based rate limiting for unauthenticated requests.
            // Extract from x-forwarded-for or peer address.
            let ip = req
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            format!("rate:api:ip:{ip}")
        }
    };

    let config = RateLimitConfig {
        key_prefix: rate_key,
        limit: 30,
        window_secs: 60,
    };
    if let Err(err) = check_rate_limit(&state.redis, &config).await {
        return axum::response::IntoResponse::into_response(err);
    }

    next.run(req).await
}
