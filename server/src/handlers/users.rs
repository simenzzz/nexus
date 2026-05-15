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
use crate::middleware::rate_limit::{auth_login_key, auth_register_key, check_rate_limit, RateLimitConfig};
use crate::models::user::CreateUser;
use crate::validation;
use crate::AppState;

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

fn build_refresh_cookie_value(value: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "refresh_token={value}; HttpOnly; Path=/api/auth; SameSite=Strict; Max-Age=604800{secure_flag}"
    )
}

fn clear_refresh_cookie_value(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!("refresh_token=; HttpOnly; Path=/api/auth; SameSite=Strict; Max-Age=0{secure_flag}")
}

fn extract_refresh_from_cookies(headers: &HeaderMap) -> Option<String> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| {
            let pair = c.trim();
            pair.strip_prefix("refresh_token=")
                .map(|v| v.to_string())
        })
}

fn extract_ip(connect_info: &ConnectInfo<std::net::SocketAddr>) -> String {
    connect_info.0.ip().to_string()
}

fn set_cookie_header(value: String) -> HeaderValue {
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn auth_response(body: Value, cookie: String) -> axum::response::Response {
    (
        StatusCode::OK,
        [(SET_COOKIE, set_cookie_header(cookie))],
        Json(body),
    )
        .into_response()
}

pub async fn create_user(
    State(state): State<AppState>,
    connect_info: ConnectInfo<std::net::SocketAddr>,
    Json(input): Json<CreateUser>,
) -> Result<axum::response::Response, AppError> {
    // Per-IP rate limit: 3 per hour
    let ip = extract_ip(&connect_info);
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
    Ok(auth_response(
        json!({ "access_token": access_token, "user": user_resp }),
        build_refresh_cookie_value(&refresh_token, state.config.secure_cookies),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    connect_info: ConnectInfo<std::net::SocketAddr>,
    Json(input): Json<LoginRequest>,
) -> Result<axum::response::Response, AppError> {
    // Per-IP rate limit: 10 per minute
    let ip = extract_ip(&connect_info);
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
    Ok(auth_response(
        json!({ "access_token": access_token, "user": user_resp }),
        build_refresh_cookie_value(&refresh_token, state.config.secure_cookies),
    ))
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let old_refresh = extract_refresh_from_cookies(&headers)
        .ok_or_else(|| AppError::Unauthorized("No refresh token".into()))?;

    let user_id = refresh::consume_refresh_token(&state.redis, &old_refresh)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid refresh token".into()))?;

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

    Ok(auth_response(
        json!({ "access_token": access_token }),
        build_refresh_cookie_value(&new_refresh, state.config.secure_cookies),
    ))
}

pub async fn ws_ticket(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, AppError> {
    let ticket = ws_ticket::generate_ticket();
    ws_ticket::store_ticket(&state.redis, &ticket, &auth.0.sub).await?;
    Ok(Json(json!({ "ticket": ticket })))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    _auth: AuthUser,
) -> Result<axum::response::Response, AppError> {
    if let Some(old_refresh) = extract_refresh_from_cookies(&headers) {
        let _ = refresh::delete_refresh_token(&state.redis, &old_refresh).await;
    }

    Ok(auth_response(
        json!({ "status": "logged_out" }),
        clear_refresh_cookie_value(state.config.secure_cookies),
    ))
}

pub async fn me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, AppError> {
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
