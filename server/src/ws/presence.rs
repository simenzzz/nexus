use deadpool_redis::Pool;

use crate::error::AppError;

pub async fn set_status(redis: &Pool, user_id: &str, status: &str) -> Result<(), AppError> {
    let key = format!("presence:{user_id}");
    let mut conn = redis.get().await?;
    redis::cmd("SET")
        .arg(&key)
        .arg(status)
        .arg("EX")
        .arg(300i64) // 5 min TTL, refreshed by heartbeat
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

pub async fn set_online_with_ttl(
    redis: &Pool,
    user_id: &str,
    ttl_secs: i64,
) -> Result<(), AppError> {
    let key = format!("presence:{user_id}");
    let mut conn = redis.get().await?;
    redis::cmd("SET")
        .arg(&key)
        .arg("online")
        .arg("EX")
        .arg(ttl_secs)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

pub async fn get_status(redis: &Pool, user_id: &str) -> Result<String, AppError> {
    let key = format!("presence:{user_id}");
    let mut conn = redis.get().await?;
    let status: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
    Ok(status.unwrap_or_else(|| "offline".to_string()))
}

pub async fn set_offline(redis: &Pool, user_id: &str) -> Result<(), AppError> {
    let key = format!("presence:{user_id}");
    let mut conn = redis.get().await?;
    redis::cmd("DEL")
        .arg(&key)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Window during which a single user can fan out at most one presence
/// transition type (online/idle/offline) to their audience. Caps the
/// amplification of a reconnect-loop client into a per-co-member event storm
/// without letting an idle broadcast suppress the later idle -> online recovery.
/// Set via `SET NX EX`, so the lock self-evicts and we never need to collect.
const FLAP_LOCK_TTL_SECS: i64 = 30;

/// Try to claim the per-user presence broadcast lock. Returns `true` if the
/// caller may fan out a presence transition to the audience, `false` if a
/// recent broadcast already used this user's slot.
///
/// Fails OPEN on Redis errors: a Redis outage shouldn't suppress legitimate
/// presence events. The grace period + multi-tab guard upstream already
/// shape the common-case traffic; this is the third layer specifically for
/// hostile reconnect loops.
pub async fn try_claim_flap_slot(redis: &Pool, user_id: &str, status: &str) -> bool {
    let key = format!("presence_flap:{user_id}:{status}");
    let mut conn = match redis.get().await {
        Ok(c) => c,
        Err(_) => return true,
    };
    // SET key 1 NX EX 30 → "OK" if set, nil if key already exists.
    let result: Result<Option<String>, _> = redis::cmd("SET")
        .arg(&key)
        .arg("1")
        .arg("NX")
        .arg("EX")
        .arg(FLAP_LOCK_TTL_SECS)
        .query_async(&mut conn)
        .await;
    matches!(result, Ok(Some(_)))
}
