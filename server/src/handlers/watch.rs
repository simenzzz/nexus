use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::Json;
use deadpool_redis::redis::AsyncCommands;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::models::channel::ChannelType;
use crate::repositories::Repos;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct RecommendationsQuery {
    /// Capped to a small range to keep the graph traversal cheap.
    pub limit: Option<u32>,
}

const REC_CACHE_TTL_SECS: u64 = 60;
const REC_DEFAULT_LIMIT: u32 = 10;
const REC_MAX_LIMIT: u32 = 50;

/// Circuit breaker for the recommendations endpoint. The endpoint depends on
/// Redis for caching; when Redis is unreachable every request falls through to
/// the expensive 2-hop Surreal traversal. Letting that happen on every request
/// during a Redis outage turns one degraded dependency into two. After
/// `BREAKER_TRIP_THRESHOLD` consecutive Redis failures we trip the breaker
/// for `BREAKER_COOLDOWN_MS` and return 503 instead, giving Surreal headroom.
///
/// Scope is intentionally process-global (static atomics) — each app instance
/// in a horizontally-scaled deploy gets its own breaker, so a degraded
/// dependency in one node doesn't punish traffic on healthy nodes. The
/// threshold is high enough that a small handful of unrelated transient
/// failures won't trip it, but low enough to react to a sustained outage in
/// under a few seconds of typical traffic. Trips emit a metric so dashboards
/// can alert on `rec_breaker_trips_total`.
const BREAKER_TRIP_THRESHOLD: u8 = 10;
const BREAKER_COOLDOWN_MS: u64 = 30_000;
static REC_BREAKER_FAILURES: AtomicU8 = AtomicU8::new(0);
static REC_BREAKER_TRIPPED_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        // Fail closed on clock failure: a stuck SystemTime should NOT be
        // treated as "before the trip deadline" because the deadline itself
        // was computed from this same clock. Returning u64::MAX makes
        // `breaker_open()` return true and trips the safe path.
        .unwrap_or(u64::MAX)
}

fn breaker_open() -> bool {
    now_ms() < REC_BREAKER_TRIPPED_UNTIL_MS.load(Ordering::Relaxed)
}

fn breaker_record_failure() {
    let failures = REC_BREAKER_FAILURES
        .fetch_add(1, Ordering::Relaxed)
        .saturating_add(1);
    if failures >= BREAKER_TRIP_THRESHOLD {
        let until = now_ms().saturating_add(BREAKER_COOLDOWN_MS);
        REC_BREAKER_TRIPPED_UNTIL_MS.store(until, Ordering::Relaxed);
        crate::metrics::record_rec_breaker_trip();
        tracing::warn!(
            consecutive_failures = failures,
            cooldown_ms = BREAKER_COOLDOWN_MS,
            "recommendations breaker tripped — Redis appears unhealthy"
        );
    }
}

fn breaker_record_success() {
    if breaker_open() {
        return;
    }
    REC_BREAKER_FAILURES.store(0, Ordering::Relaxed);
    REC_BREAKER_TRIPPED_UNTIL_MS.store(0, Ordering::Relaxed);
}

/// `GET /api/channels/:channel_id/watch/recommendations?limit=10` — surfaces
/// videos that other members of the user's servers have watched but the user
/// hasn't. Cached in Redis for 60s per user to amortize the 2-hop traversal.
pub async fn get_recommendations(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<RecommendationsQuery>,
) -> Result<Json<Value>, AppError> {
    authorize_watch_member(&state.repos, &channel_id, &claims.sub).await?;

    let limit = query
        .limit
        .unwrap_or(REC_DEFAULT_LIMIT)
        .clamp(1, REC_MAX_LIMIT);

    // If the breaker is open we fail fast — protects Surreal from a stampede
    // when Redis is down. Clients can retry after the cooldown.
    if breaker_open() {
        return Err(AppError::ServiceUnavailable(
            "Recommendations temporarily unavailable".into(),
        ));
    }

    let cache_key = format!("rec:user:{}:{limit}", claims.sub);

    // Cache read. Hits return early; misses proceed to the traversal. Redis
    // failures (pool fetch or GET) count toward the breaker — three in a row
    // and we stop hitting Surreal until the cooldown expires.
    let mut redis_ok = false;
    match state.redis.get().await {
        Ok(mut conn) => match conn.get::<_, Option<String>>(&cache_key).await {
            Ok(Some(json)) => {
                breaker_record_success();
                if let Ok(value) = serde_json::from_str::<Value>(&json) {
                    return Ok(Json(value));
                }
                redis_ok = true;
            }
            Ok(None) => {
                redis_ok = true;
            }
            Err(e) => {
                tracing::debug!(error = %e, "rec cache read failed");
                breaker_record_failure();
                if breaker_open() {
                    return Err(AppError::ServiceUnavailable(
                        "Recommendations temporarily unavailable".into(),
                    ));
                }
            }
        },
        Err(e) => {
            tracing::debug!(error = %e, "rec cache pool get failed");
            breaker_record_failure();
            if breaker_open() {
                return Err(AppError::ServiceUnavailable(
                    "Recommendations temporarily unavailable".into(),
                ));
            }
        }
    }
    if redis_ok {
        breaker_record_success();
    }

    let recs = state
        .repos
        .recommendations
        .for_user(&claims.sub, limit)
        .await?;

    let payload = json!({
        "channel_id": channel_id,
        "recommendations": recs,
    });

    // Cache write — best-effort; ignore errors so a Redis blip doesn't block
    // the response. The TTL prevents unbounded staleness.
    if let Ok(mut conn) = state.redis.get().await {
        if let Ok(json_str) = serde_json::to_string(&payload) {
            let _: Result<(), _> = conn.set_ex(&cache_key, json_str, REC_CACHE_TTL_SECS).await;
        }
    }

    Ok(Json(payload))
}

/// Same fail-closed shape as `authorize_member` in whiteboards.rs — single
/// generic Forbidden whether the channel is missing, the wrong type, or the
/// user isn't a member. Details only in trace logs.
async fn authorize_watch_member(
    repos: &Repos,
    channel_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let channel = match repos.channels.find_by_id(channel_id).await? {
        Some(c) => c,
        None => {
            tracing::debug!(channel_id = %channel_id, user_id = %user_id, "watch auth: channel missing");
            return Err(AppError::Forbidden(
                "Not authorized for this watch room".into(),
            ));
        }
    };
    if !matches!(channel.channel_type, ChannelType::Watch) {
        tracing::debug!(channel_id = %channel_id, user_id = %user_id, "watch auth: wrong channel type");
        return Err(AppError::Forbidden(
            "Not authorized for this watch room".into(),
        ));
    }
    let server_key = channel.server.key().to_string();
    let is_member = repos.servers.is_member(&server_key, user_id).await?;
    if !is_member {
        tracing::debug!(channel_id = %channel_id, user_id = %user_id, "watch auth: non-member");
        return Err(AppError::Forbidden(
            "Not authorized for this watch room".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_does_not_clear_open_breaker_cooldown() {
        REC_BREAKER_FAILURES.store(BREAKER_TRIP_THRESHOLD, Ordering::Relaxed);
        REC_BREAKER_TRIPPED_UNTIL_MS.store(
            now_ms().saturating_add(BREAKER_COOLDOWN_MS),
            Ordering::Relaxed,
        );

        breaker_record_success();

        assert!(breaker_open());
        assert_eq!(
            REC_BREAKER_FAILURES.load(Ordering::Relaxed),
            BREAKER_TRIP_THRESHOLD
        );

        REC_BREAKER_FAILURES.store(0, Ordering::Relaxed);
        REC_BREAKER_TRIPPED_UNTIL_MS.store(0, Ordering::Relaxed);
    }
}
