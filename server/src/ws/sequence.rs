use deadpool_redis::Pool;

use crate::error::AppError;

pub async fn next_seq(redis: &Pool, channel_id: &str) -> Result<u64, AppError> {
    let key = format!("channel_seq:{channel_id}");
    let mut conn = redis.get().await?;
    let seq: u64 = redis::cmd("INCR")
        .arg(&key)
        .query_async(&mut conn)
        .await?;
    // Refresh TTL on each increment (7 days)
    let _ = redis::cmd("EXPIRE")
        .arg(&key)
        .arg(86400i64 * 7)
        .query_async::<()>(&mut conn)
        .await;
    Ok(seq)
}

pub async fn current_seq(redis: &Pool, channel_id: &str) -> Result<u64, AppError> {
    let key = format!("channel_seq:{channel_id}");
    let mut conn = redis.get().await?;
    let seq: Option<u64> = redis::cmd("GET")
        .arg(&key)
        .query_async(&mut conn)
        .await?;
    Ok(seq.unwrap_or(0))
}
