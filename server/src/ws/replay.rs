use deadpool_redis::Pool;

use crate::error::AppError;

const MAX_REPLAY: i64 = 500;

pub async fn store_message(
    redis: &Pool,
    channel_id: &str,
    seq: u64,
    serialized_msg: &str,
) -> Result<(), AppError> {
    let key = format!("channel_replay:{channel_id}");
    let mut conn = redis.get().await?;

    redis::cmd("ZADD")
        .arg(&key)
        .arg(seq)
        .arg(serialized_msg)
        .query_async::<()>(&mut conn)
        .await?;

    // Trim: only remove excess messages when buffer exceeds max
    let count: i64 = redis::cmd("ZCARD")
        .arg(&key)
        .query_async(&mut conn)
        .await?;

    if count > MAX_REPLAY {
        redis::cmd("ZREMRANGEBYRANK")
            .arg(&key)
            .arg(0)
            .arg(count - MAX_REPLAY - 1)
            .query_async::<()>(&mut conn)
            .await?;
    }

    // Set a 7-day TTL so inactive channels don't accumulate forever
    redis::cmd("EXPIRE")
        .arg(&key)
        .arg(86400i64 * 7)
        .query_async::<()>(&mut conn)
        .await?;

    Ok(())
}

/// Returns None if the gap is too large (>500 messages), triggering a resync.
pub async fn get_missed_messages(
    redis: &Pool,
    channel_id: &str,
    last_seq: u64,
) -> Result<Option<Vec<String>>, AppError> {
    let key = format!("channel_replay:{channel_id}");
    let mut conn = redis.get().await?;

    let count: i64 = redis::cmd("ZCOUNT")
        .arg(&key)
        .arg(last_seq + 1)
        .arg("+inf")
        .query_async(&mut conn)
        .await?;

    if count > MAX_REPLAY {
        return Ok(None);
    }

    let messages: Vec<String> = redis::cmd("ZRANGEBYSCORE")
        .arg(&key)
        .arg(last_seq + 1)
        .arg("+inf")
        .query_async(&mut conn)
        .await?;

    Ok(Some(messages))
}
