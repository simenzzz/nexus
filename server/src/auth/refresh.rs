use deadpool_redis::Pool;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::AppError;

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub async fn store_refresh_token(
    redis: &Pool,
    token: &str,
    user_id: &str,
    ttl_days: u64,
) -> Result<(), AppError> {
    let key = format!("refresh:{}", hash_token(token));
    let mut conn = redis.get().await?;
    redis::cmd("SET")
        .arg(&key)
        .arg(user_id)
        .arg("EX")
        .arg(ttl_days * 86400)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Atomically consume a refresh token (GETDEL) — prevents race-condition reuse.
pub async fn consume_refresh_token(
    redis: &Pool,
    token: &str,
) -> Result<Option<String>, AppError> {
    let key = format!("refresh:{}", hash_token(token));
    let mut conn = redis.get().await?;
    let user_id: Option<String> = redis::cmd("GETDEL")
        .arg(&key)
        .query_async(&mut conn)
        .await?;
    Ok(user_id)
}

pub async fn delete_refresh_token(redis: &Pool, token: &str) -> Result<(), AppError> {
    let key = format!("refresh:{}", hash_token(token));
    let mut conn = redis.get().await?;
    redis::cmd("DEL")
        .arg(&key)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}
