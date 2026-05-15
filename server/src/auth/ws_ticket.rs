use deadpool_redis::Pool;
use rand::RngCore;

use crate::error::AppError;

pub fn generate_ticket() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub async fn store_ticket(
    redis: &Pool,
    ticket: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let key = format!("ws_ticket:{ticket}");
    let mut conn = redis.get().await?;
    redis::cmd("SET")
        .arg(&key)
        .arg(user_id)
        .arg("EX")
        .arg(30i64)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

pub async fn consume_ticket(
    redis: &Pool,
    ticket: &str,
) -> Result<Option<String>, AppError> {
    let key = format!("ws_ticket:{ticket}");
    let mut conn = redis.get().await?;
    // GETDEL — atomic get + delete (requires Redis 6.2+)
    let user_id: Option<String> = redis::cmd("GETDEL")
        .arg(&key)
        .query_async(&mut conn)
        .await?;
    Ok(user_id)
}
