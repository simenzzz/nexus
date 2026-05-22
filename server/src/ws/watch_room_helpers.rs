use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use crate::ws::protocol::ServerMessage;
use crate::ws::watch_types::QueueItemSummary;

/// Order the queue by score desc; ties broken by id asc so the in-memory
/// order matches what `WatchRepo::list_queue` returns from the DB
/// (`score DESC, created_at ASC`).
pub(super) fn sort_queue(queue: &mut [QueueItemSummary]) {
    queue.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
}

/// YouTube video ids are exactly 11 characters from the URL-safe base64
/// alphabet. We use this both as input validation (against pasted URLs) and
/// as a guard against arbitrary payloads getting persisted.
pub(super) fn is_valid_youtube_id(id: &str) -> bool {
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Bounded structural check for emoji reactions. The WS boundary already
/// trims and caps at 32 bytes; this is defense-in-depth so a future code
/// path that bypasses connection.rs can't fan junk out to every viewer.
///
/// We allow up to 32 bytes (enough for flag sequences / ZWJ family emoji
/// — ZWJ U+200D is intentionally permitted) but reject:
///   - empty payloads
///   - ASCII / Unicode control characters
///   - line / paragraph separators that break out of a single-line render
///   - bidi formatting codepoints that can flip surrounding usernames or
///     timestamps in the reactions panel ("Trojan Source"-style spoofing)
pub(super) fn is_valid_reaction_emoji(s: &str) -> bool {
    if s.is_empty() || s.len() > 32 {
        return false;
    }
    !s.chars().any(|c| {
        c.is_control()
            || c == '\u{2028}'                     // LINE SEPARATOR
            || c == '\u{2029}'                     // PARAGRAPH SEPARATOR
            || c == '\u{200E}' || c == '\u{200F}'  // LRM / RLM
            || ('\u{202A}'..='\u{202E}').contains(&c) // LRE/RLE/PDF/LRO/RLO
            || ('\u{2066}'..='\u{2069}').contains(&c) // LRI/RLI/FSI/PDI
            || c == '\u{FEFF}' // BOM / ZWNBSP
    })
}

pub(super) fn send_error(tx: &mpsc::Sender<String>, channel_id: &str, code: &str, message: &str) {
    let _ = tx.try_send(
        ServerMessage::WatchError {
            channel_id: channel_id.to_string(),
            code: code.to_string(),
            message: message.to_string(),
        }
        .to_json(),
    );
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_youtube_id_passes() {
        assert!(is_valid_youtube_id("dQw4w9WgXcQ"));
        assert!(is_valid_youtube_id("_-aaaaaaaaa"));
    }

    #[test]
    fn invalid_youtube_id_rejected() {
        assert!(!is_valid_youtube_id("tooshort"));
        assert!(!is_valid_youtube_id("waytoolongforanid"));
        assert!(!is_valid_youtube_id("badchar!!!!"));
        assert!(!is_valid_youtube_id(""));
    }

    #[test]
    fn reaction_emoji_validation() {
        assert!(is_valid_reaction_emoji("🎉"));
        assert!(is_valid_reaction_emoji("👍"));
        assert!(is_valid_reaction_emoji("🇺🇸"));
        assert!(is_valid_reaction_emoji("👨‍👩‍👧"));
        assert!(!is_valid_reaction_emoji(""));
        let long = "🎉".repeat(9);
        assert!(!is_valid_reaction_emoji(&long));
        assert!(!is_valid_reaction_emoji("hi\n"));
        assert!(!is_valid_reaction_emoji("\u{0007}"));
        assert!(!is_valid_reaction_emoji("a\u{2028}"));
        assert!(!is_valid_reaction_emoji("\u{202E}🎉"));
        assert!(!is_valid_reaction_emoji("🎉\u{200F}"));
        assert!(!is_valid_reaction_emoji("\u{2066}x"));
        assert!(!is_valid_reaction_emoji("\u{FEFF}"));
        assert!(is_valid_reaction_emoji("\u{1F468}\u{200D}\u{1F469}"));
    }

    #[test]
    fn sort_queue_orders_by_score_desc() {
        let mut q = vec![
            QueueItemSummary {
                id: "a".into(),
                video_id: "v1".into(),
                title: "".into(),
                duration_ms: 0,
                thumbnail_url: None,
                added_by: "u".into(),
                score: 1,
            },
            QueueItemSummary {
                id: "b".into(),
                video_id: "v2".into(),
                title: "".into(),
                duration_ms: 0,
                thumbnail_url: None,
                added_by: "u".into(),
                score: 5,
            },
            QueueItemSummary {
                id: "c".into(),
                video_id: "v3".into(),
                title: "".into(),
                duration_ms: 0,
                thumbnail_url: None,
                added_by: "u".into(),
                score: -2,
            },
        ];
        sort_queue(&mut q);
        assert_eq!(q[0].id, "b");
        assert_eq!(q[1].id, "a");
        assert_eq!(q[2].id, "c");
    }
}
