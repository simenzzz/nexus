use deadpool_redis::Pool;

use crate::error::AppError;

pub async fn set_status(
    redis: &Pool,
    user_id: &str,
    status: &str,
) -> Result<(), AppError> {
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
    let status: Option<String> = redis::cmd("GET")
        .arg(&key)
        .query_async(&mut conn)
        .await?;
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
