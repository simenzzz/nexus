use deadpool_redis::Pool;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub key_prefix: String,
    pub limit: u64,
    pub window_secs: u64,
}

/// Token bucket rate limiter using a Redis Lua script for atomicity.
/// Returns Ok(()) if allowed, or Err(AppError::RateLimited) with retry_after seconds.
pub async fn check_rate_limit(redis: &Pool, config: &RateLimitConfig) -> Result<(), AppError> {
    let mut conn = redis.get().await?;
    let key = config.key_prefix.clone();

    let lua_script = r#"
        local key = KEYS[1]
        local limit = tonumber(ARGV[1])
        local window = tonumber(ARGV[2])
        local current = tonumber(redis.call('GET', key) or '0')

        if current >= limit then
            local ttl = redis.call('TTL', key)
            return tonumber(ttl > 0 and ttl or window)
        end

        current = redis.call('INCR', key)
        if current == 1 then
            redis.call('EXPIRE', key, window)
        end

        return tonumber(0)
    "#;

    let retry_after: i64 = redis::cmd("EVAL")
        .arg(lua_script)
        .arg(1)
        .arg(&key)
        .arg(config.limit)
        .arg(config.window_secs)
        .query_async(&mut conn)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Rate limiter Redis error — failing closed");
            AppError::Internal("Service temporarily unavailable".into())
        })?;

    if retry_after > 0 {
        Err(AppError::RateLimited {
            retry_after: retry_after as u64,
        })
    } else {
        Ok(())
    }
}

pub fn message_send_key(user_id: &str, channel_id: &str) -> String {
    format!("rate:msg:{user_id}:{channel_id}")
}

pub fn api_general_key(user_id: &str) -> String {
    format!("rate:api:{user_id}")
}

pub fn ws_connect_key(user_id: &str) -> String {
    format!("rate:ws:{user_id}")
}

pub fn friend_request_key(user_id: &str) -> String {
    format!("rate:friend:{user_id}")
}

pub fn auth_login_key(ip: &str) -> String {
    format!("rate:login:{ip}")
}

pub fn auth_register_key(ip: &str) -> String {
    format!("rate:register:{ip}")
}

pub fn auth_refresh_key(user_id: &str) -> String {
    format!("rate:refresh:{user_id}")
}

pub fn auth_ws_ticket_key(user_id: &str) -> String {
    format!("rate:wsticket:{user_id}")
}

pub fn whiteboard_update_key(user_id: &str, channel_id: &str) -> String {
    format!("rate:wb:upd:{user_id}:{channel_id}")
}

pub fn whiteboard_awareness_key(user_id: &str, channel_id: &str) -> String {
    format!("rate:wb:aw:{user_id}:{channel_id}")
}

/// Whiteboard subscribe/unsubscribe churn protection — prevents abusing the
/// (uncached) channel/server membership lookup that runs on every subscribe.
pub fn whiteboard_subscribe_key(user_id: &str) -> String {
    format!("rate:wb:sub:{user_id}")
}

/// Per-resource subscribe key — also rate-limits subscribe churn against a
/// specific channel.
pub fn collab_subscribe_key(user_id: &str) -> String {
    format!("rate:collab:sub:{user_id}")
}

/// Watch-room playback control (play/pause/seek). Leader-only path —
/// non-leaders are rejected before they hit this. 10/sec is generous given
/// each click maps to one event and the leader is a single user per room.
pub fn watch_playback_control_key(user_id: &str, channel_id: &str) -> String {
    format!("rate:watch:pb:{user_id}:{channel_id}")
}

/// Queue mutation ops (add/remove/vote/skip). Tight enough to thwart abuse
/// but loose enough that genuine voting flurries pass.
pub fn watch_queue_op_key(user_id: &str, channel_id: &str) -> String {
    format!("rate:watch:q:{user_id}:{channel_id}")
}

/// Live reactions — capped low because each reaction is broadcast to every
/// other viewer (fan-out amplification).
pub fn watch_reaction_key(user_id: &str, channel_id: &str) -> String {
    format!("rate:watch:rx:{user_id}:{channel_id}")
}
