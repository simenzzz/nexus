use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::jwt::create_access_token;
use crate::auth::middleware::AuthUser;
use crate::auth::password;
use crate::auth::refresh;
use crate::auth::ws_ticket;
use crate::error::AppError;
use crate::middleware::csrf;
use crate::middleware::rate_limit::{
    auth_login_key, auth_refresh_key, auth_register_key, auth_ws_ticket_key, check_rate_limit,
    RateLimitConfig,
};
use crate::models::user::CreateUser;
use crate::validation;
use crate::AppState;
use std::net::IpAddr;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

fn user_to_response(user: &crate::models::user::User) -> UserResponse {
    UserResponse {
        id: user
            .id
            .as_ref()
            .map(|rid| rid.key().to_string())
            .unwrap_or_default(),
        username: user.username.clone(),
        display_name: user.display_name.clone(),
        avatar_url: user.avatar_url.clone(),
    }
}

fn user_id_from_record(user: &crate::models::user::User) -> String {
    user.id
        .as_ref()
        .map(|rid| rid.key().to_string())
        .unwrap_or_default()
}

/// Cookie name for the refresh token. The `__Host-` prefix in secure mode
/// requires Secure + Path=/ and forbids Domain, blocking parent-domain
/// cookie planting. NB: `Path` for `__Host-` cookies MUST be `/`, not
/// `/api/auth` — browsers reject the cookie otherwise. We pay a small
/// scope expansion in production for the planting protection.
const REFRESH_COOKIE_PLAIN: &str = "refresh_token";
const REFRESH_COOKIE_HOST: &str = "__Host-refresh_token";

fn refresh_cookie_name(secure: bool) -> &'static str {
    if secure {
        REFRESH_COOKIE_HOST
    } else {
        REFRESH_COOKIE_PLAIN
    }
}

fn build_refresh_cookie_value(value: &str, secure: bool) -> String {
    let name = refresh_cookie_name(secure);
    if secure {
        format!("{name}={value}; HttpOnly; Path=/; SameSite=Strict; Max-Age=604800; Secure")
    } else {
        format!("{name}={value}; HttpOnly; Path=/api/auth; SameSite=Strict; Max-Age=604800")
    }
}

fn clear_refresh_cookie_by_name(name: &str, secure: bool) -> String {
    if secure {
        format!("{name}=; HttpOnly; Path=/; SameSite=Strict; Max-Age=0; Secure")
    } else {
        format!("{name}=; HttpOnly; Path=/api/auth; SameSite=Strict; Max-Age=0")
    }
}

fn clear_refresh_cookie_values(secure: bool) -> Vec<String> {
    if secure {
        vec![
            clear_refresh_cookie_by_name(REFRESH_COOKIE_HOST, true),
            clear_refresh_cookie_by_name(REFRESH_COOKIE_PLAIN, true),
            clear_refresh_cookie_by_name(REFRESH_COOKIE_PLAIN, false),
        ]
    } else {
        vec![clear_refresh_cookie_by_name(REFRESH_COOKIE_PLAIN, false)]
    }
}

fn extract_refresh_from_cookies(headers: &HeaderMap, secure: bool) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    let candidates: &[&str] = if secure {
        &[REFRESH_COOKIE_HOST]
    } else {
        &[REFRESH_COOKIE_PLAIN]
    };
    for candidate in candidates {
        let prefix = format!("{candidate}=");
        for part in raw.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix(&prefix) {
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn trusted_proxy_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    }
}

fn parse_forwarded_for(value: &str) -> Option<IpAddr> {
    let first = value.split(',').next()?.trim().trim_matches('"');
    let without_brackets = first
        .strip_prefix('[')
        .and_then(|s| s.split(']').next())
        .unwrap_or(first);
    if let Ok(ip) = without_brackets.parse::<IpAddr>() {
        return Some(ip);
    }
    let host = without_brackets
        .rsplit_once(':')
        .filter(|(_, port)| port.parse::<u16>().is_ok())
        .map(|(host, _)| host)
        .unwrap_or(without_brackets);
    host.parse::<IpAddr>().ok()
}

fn parse_forwarded_header(value: &str) -> Option<IpAddr> {
    value.split(',').find_map(|entry| {
        entry.split(';').find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            if !key.eq_ignore_ascii_case("for") {
                return None;
            }
            parse_forwarded_for(value.trim())
        })
    })
}

fn extract_ip(headers: &HeaderMap, connect_info: &ConnectInfo<std::net::SocketAddr>) -> String {
    let peer_ip = connect_info.0.ip();
    if trusted_proxy_ip(peer_ip) {
        if let Some(ip) = headers
            .get("forwarded")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_forwarded_header)
            .or_else(|| {
                headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_forwarded_for)
            })
            .or_else(|| {
                headers
                    .get("x-real-ip")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<IpAddr>().ok())
            })
        {
            return ip.to_string();
        }
    }
    peer_ip.to_string()
}

fn set_cookie_header(value: String) -> HeaderValue {
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Build a response that emits one or more Set-Cookie headers. axum will
/// serialize repeated header insertions as separate Set-Cookie lines (the
/// HTTP spec's only multi-valued header).
fn auth_response_with_cookies(body: Value, cookies: Vec<String>) -> axum::response::Response {
    let mut response = (StatusCode::OK, Json(body)).into_response();
    let headers = response.headers_mut();
    for cookie in cookies {
        headers.append(SET_COOKIE, set_cookie_header(cookie));
    }
    response
}

fn refresh_cookies(refresh_token: &str, secure: bool) -> Vec<String> {
    vec![
        build_refresh_cookie_value(refresh_token, secure),
        csrf::build_cookie_value(&csrf::generate_token(), secure),
    ]
}

pub async fn create_user(
    State(state): State<AppState>,
    connect_info: ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<CreateUser>,
) -> Result<axum::response::Response, AppError> {
    // Per-IP rate limit: 3 per hour
    let ip = extract_ip(&headers, &connect_info);
    check_rate_limit(
        &state.redis,
        &RateLimitConfig {
            key_prefix: auth_register_key(&ip),
            limit: 3,
            window_secs: 3600,
        },
    )
    .await?;

    validation::validate_username(&input.username)?;
    validation::validate_password(&input.password)?;

    if state
        .repos
        .users
        .find_by_username(&input.username)
        .await?
        .is_some()
    {
        return Err(AppError::BadRequest("Username already taken".into()));
    }

    let hash = password::hash_password(&input.password).await?;
    let user = state.repos.users.create(input, hash).await?;
    let user_id = user_id_from_record(&user);

    let access_token = create_access_token(
        &user_id,
        &state.config.jwt_secret,
        state.config.access_token_expiry_minutes,
    )?;

    let refresh_token = refresh::generate_refresh_token();
    refresh::store_refresh_token(
        &state.redis,
        &refresh_token,
        &user_id,
        state.config.refresh_token_expiry_days,
    )
    .await?;

    let user_resp = user_to_response(&user);
    Ok(auth_response_with_cookies(
        json!({ "access_token": access_token, "user": user_resp }),
        refresh_cookies(&refresh_token, state.config.secure_cookies),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    connect_info: ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<LoginRequest>,
) -> Result<axum::response::Response, AppError> {
    // Per-IP rate limit: 10 per minute
    let ip = extract_ip(&headers, &connect_info);
    check_rate_limit(
        &state.redis,
        &RateLimitConfig {
            key_prefix: auth_login_key(&ip),
            limit: 10,
            window_secs: 60,
        },
    )
    .await?;

    let user_with_pw = state
        .repos
        .users
        .find_by_username(&input.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;

    if !password::verify_password(&input.password, &user_with_pw.password_hash).await? {
        return Err(AppError::Unauthorized("Invalid credentials".into()));
    }

    let user_id = user_with_pw
        .id
        .as_ref()
        .map(|rid| rid.key().to_string())
        .unwrap_or_default();

    let user: crate::models::user::User = user_with_pw.into();
    let access_token = create_access_token(
        &user_id,
        &state.config.jwt_secret,
        state.config.access_token_expiry_minutes,
    )?;

    let refresh_token = refresh::generate_refresh_token();
    refresh::store_refresh_token(
        &state.redis,
        &refresh_token,
        &user_id,
        state.config.refresh_token_expiry_days,
    )
    .await?;

    let user_resp = user_to_response(&user);
    Ok(auth_response_with_cookies(
        json!({ "access_token": access_token, "user": user_resp }),
        refresh_cookies(&refresh_token, state.config.secure_cookies),
    ))
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let old_refresh = extract_refresh_from_cookies(&headers, state.config.secure_cookies)
        .ok_or_else(|| AppError::Unauthorized("No refresh token".into()))?;

    let user_id = refresh::get_refresh_token_user(&state.redis, &old_refresh)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid refresh token".into()))?;

    // Per-user rate limit (1 / 5s): detects leaked-token replay.
    check_rate_limit(
        &state.redis,
        &RateLimitConfig {
            key_prefix: auth_refresh_key(&user_id),
            limit: 1,
            window_secs: 5,
        },
    )
    .await
    .inspect_err(|_| {
        tracing::warn!(
            event = "auth_rate_limited",
            endpoint = "/api/auth/refresh",
            user = %user_id,
            "rate-limit hit on refresh"
        );
    })?;

    let consumed_user_id = refresh::consume_refresh_token(&state.redis, &old_refresh)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid refresh token".into()))?;
    if consumed_user_id != user_id {
        return Err(AppError::Unauthorized("Invalid refresh token".into()));
    }

    let access_token = create_access_token(
        &user_id,
        &state.config.jwt_secret,
        state.config.access_token_expiry_minutes,
    )?;

    let new_refresh = refresh::generate_refresh_token();
    refresh::store_refresh_token(
        &state.redis,
        &new_refresh,
        &user_id,
        state.config.refresh_token_expiry_days,
    )
    .await?;

    Ok(auth_response_with_cookies(
        json!({ "access_token": access_token }),
        refresh_cookies(&new_refresh, state.config.secure_cookies),
    ))
}

#[derive(Debug, Serialize)]
pub struct WsTicketResponse {
    pub ticket: String,
    pub nonce: String,
}

pub async fn ws_ticket(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<WsTicketResponse>, AppError> {
    // Per-user rate limit on ticket issuance. Allows small multi-tab/reconnect
    // bursts without letting a client mint unbounded one-shot credentials.
    check_rate_limit(
        &state.redis,
        &RateLimitConfig {
            key_prefix: auth_ws_ticket_key(&auth.0.sub),
            limit: 5,
            window_secs: 10,
        },
    )
    .await
    .inspect_err(|_| {
        tracing::warn!(
            event = "auth_rate_limited",
            endpoint = "/api/auth/ws-ticket",
            user = %auth.0.sub,
            "rate-limit hit on ws-ticket"
        );
    })?;

    let ticket = ws_ticket::generate_ticket();
    let nonce = ws_ticket::generate_nonce();
    ws_ticket::store_ticket(&state.redis, &ticket, &auth.0.sub, &nonce).await?;
    Ok(Json(WsTicketResponse { ticket, nonce }))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    _auth: AuthUser,
) -> Result<axum::response::Response, AppError> {
    if let Some(old_refresh) = extract_refresh_from_cookies(&headers, state.config.secure_cookies) {
        let _ = refresh::delete_refresh_token(&state.redis, &old_refresh).await;
    }

    let mut cookies = clear_refresh_cookie_values(state.config.secure_cookies);
    cookies.extend(csrf::clear_cookie_values(state.config.secure_cookies));
    Ok(auth_response_with_cookies(
        json!({ "status": "logged_out" }),
        cookies,
    ))
}

pub async fn me(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, AppError> {
    let user = state
        .repos
        .users
        .find_by_id(&auth.0.sub)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    Ok(Json(json!({ "user": user_to_response(&user) })))
}

pub async fn get_user(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let user = state
        .repos
        .users
        .find_by_id(&id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    Ok(Json(json!({ "user": user_to_response(&user) })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn headers_with_cookie(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(cookie).unwrap());
        headers
    }

    #[test]
    fn secure_refresh_cookie_extraction_ignores_plain_name() {
        let headers = headers_with_cookie("refresh_token=plain; __Host-refresh_token=host");
        assert_eq!(
            extract_refresh_from_cookies(&headers, true).as_deref(),
            Some("host")
        );

        let headers = headers_with_cookie("refresh_token=plain");
        assert!(extract_refresh_from_cookies(&headers, true).is_none());
    }

    #[test]
    fn dev_refresh_cookie_extraction_uses_plain_name() {
        let headers = headers_with_cookie("refresh_token=plain; __Host-refresh_token=host");
        assert_eq!(
            extract_refresh_from_cookies(&headers, false).as_deref(),
            Some("plain")
        );
    }

    #[test]
    fn forwarded_ip_is_used_only_from_trusted_proxy_peer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.10, 10.0.0.2"),
        );

        let trusted = ConnectInfo(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 3), 5000)));
        assert_eq!(extract_ip(&headers, &trusted), "203.0.113.10");

        let untrusted = ConnectInfo(SocketAddr::from((Ipv4Addr::new(198, 51, 100, 9), 5000)));
        assert_eq!(extract_ip(&headers, &untrusted), "198.51.100.9");
    }

    #[test]
    fn forwarded_header_parser_handles_quoted_for_value() {
        assert_eq!(
            parse_forwarded_header(r#"for="203.0.113.20";proto=https"#),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 20)))
        );
    }
}
