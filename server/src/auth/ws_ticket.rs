use deadpool_redis::Pool;
use rand::RngCore;

use crate::error::AppError;

const TICKET_TTL_SECS: i64 = 10;

pub fn generate_ticket() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn generate_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn key_uid(ticket: &str) -> String {
    format!("ws_ticket:{ticket}:uid")
}
fn key_nonce(ticket: &str) -> String {
    format!("ws_ticket:{ticket}:nonce")
}

/// Store the ticket as two sibling keys with identical TTL — no in-band
/// separator means no encoding pitfalls regardless of what shape the
/// user_id takes (SurrealDB RecordId keys can in principle contain `|`).
/// Both writes happen atomically via MULTI/EXEC.
pub async fn store_ticket(
    redis: &Pool,
    ticket: &str,
    user_id: &str,
    nonce: &str,
) -> Result<(), AppError> {
    let mut conn = redis.get().await?;
    let _: () = redis::pipe()
        .atomic()
        .cmd("SET")
        .arg(key_uid(ticket))
        .arg(user_id)
        .arg("EX")
        .arg(TICKET_TTL_SECS)
        .ignore()
        .cmd("SET")
        .arg(key_nonce(ticket))
        .arg(nonce)
        .arg("EX")
        .arg(TICKET_TTL_SECS)
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

/// Atomically consume both keys (MULTI {GETDEL, GETDEL}) and validate the
/// supplied nonce in constant time. Returns the user id on success, or
/// `None` for any failure (missing ticket, bad nonce, partial state).
/// Never tell the caller which one failed.
pub async fn consume_ticket(
    redis: &Pool,
    ticket: &str,
    supplied_nonce: &str,
) -> Result<Option<String>, AppError> {
    let mut conn = redis.get().await?;
    let (uid, nonce): (Option<String>, Option<String>) = redis::pipe()
        .atomic()
        .cmd("GETDEL")
        .arg(key_uid(ticket))
        .cmd("GETDEL")
        .arg(key_nonce(ticket))
        .query_async(&mut conn)
        .await?;

    let (Some(uid), Some(stored_nonce)) = (uid, nonce) else {
        return Ok(None);
    };
    if !constant_time_eq(stored_nonce.as_bytes(), supplied_nonce.as_bytes()) {
        return Ok(None);
    }
    Ok(Some(uid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_and_nonce_are_distinct_lengths() {
        assert_eq!(generate_ticket().len(), 64);
        assert_eq!(generate_nonce().len(), 32);
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn keys_are_namespaced() {
        assert_eq!(key_uid("xxx"), "ws_ticket:xxx:uid");
        assert_eq!(key_nonce("xxx"), "ws_ticket:xxx:nonce");
    }
}
