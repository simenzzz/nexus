use axum::body::Body;
use axum::extract::State;
use axum::http::header::COOKIE;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;

use crate::AppState;

const CSRF_COOKIE_PLAIN: &str = "csrf_token";
/// Cookie name used when `secure=true`. The `__Host-` prefix prohibits the
/// `Domain` attribute and requires `Path=/`, blocking a network attacker on
/// a parent/sibling domain from planting a same-named cookie. Browsers
/// enforce this at the cookie-jar layer.
const CSRF_COOKIE_HOST: &str = "__Host-csrf_token";
const CSRF_HEADER: &str = "x-csrf-token";

fn cookie_name(secure: bool) -> &'static str {
    if secure {
        CSRF_COOKIE_HOST
    } else {
        CSRF_COOKIE_PLAIN
    }
}

pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn build_cookie_value(value: &str, secure: bool) -> String {
    let name = cookie_name(secure);
    let secure_flag = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path=/; SameSite=Strict; Max-Age=604800{secure_flag}")
}

pub fn clear_cookie_value(secure: bool) -> String {
    let name = cookie_name(secure);
    let secure_flag = if secure { "; Secure" } else { "" };
    format!("{name}=; Path=/; SameSite=Strict; Max-Age=0{secure_flag}")
}

pub fn clear_cookie_values(secure: bool) -> Vec<String> {
    if secure {
        vec![
            clear_cookie_value(true),
            format!("{CSRF_COOKIE_PLAIN}=; Path=/; SameSite=Strict; Max-Age=0; Secure"),
        ]
    } else {
        vec![clear_cookie_value(false)]
    }
}

fn extract_csrf_cookie(headers: &axum::http::HeaderMap, secure: bool) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    let candidates: &[&str] = if secure {
        &[CSRF_COOKIE_HOST]
    } else {
        &[CSRF_COOKIE_PLAIN]
    };
    for candidate in candidates {
        let prefix = format!("{candidate}=");
        for part in raw.split(';') {
            let part = part.trim();
            if let Some(value) = part.strip_prefix(&prefix) {
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
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

/// Double-submit CSRF middleware. Mutating requests must echo the csrf_token
/// cookie value via the X-CSRF-Token header. Safe methods bypass.
pub async fn csrf_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response<Body> {
    let secure = state.config.secure_cookies;
    match *req.method() {
        Method::GET | Method::HEAD | Method::OPTIONS => return next.run(req).await,
        _ => {}
    }

    let cookie = match extract_csrf_cookie(req.headers(), secure) {
        Some(c) if !c.is_empty() => c,
        _ => return csrf_error("missing CSRF cookie"),
    };
    let header = match req
        .headers()
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(h) if !h.is_empty() => h,
        _ => return csrf_error("missing CSRF header"),
    };

    if !constant_time_eq(cookie.as_bytes(), header.as_bytes()) {
        return csrf_error("CSRF token mismatch");
    }

    next.run(req).await
}

fn csrf_error(reason: &'static str) -> Response<Body> {
    tracing::warn!(reason, "CSRF validation failed");
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({ "error": "csrf_failed" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn constant_time_eq_equal() {
        assert!(constant_time_eq(b"abc", b"abc"));
    }
    #[test]
    fn constant_time_eq_diff_value() {
        assert!(!constant_time_eq(b"abc", b"abd"));
    }
    #[test]
    fn constant_time_eq_diff_len() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
    #[test]
    fn cookie_value_includes_secure_when_requested() {
        let v = build_cookie_value("abc", true);
        assert!(v.contains("Secure"));
        assert!(v.contains("SameSite=Strict"));
    }
    #[test]
    fn cookie_value_omits_secure_in_dev() {
        let v = build_cookie_value("abc", false);
        assert!(!v.contains("Secure"));
    }

    #[test]
    fn secure_extract_ignores_plain_cookie_name() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("csrf_token=plain; __Host-csrf_token=host"),
        );
        assert_eq!(extract_csrf_cookie(&headers, true).as_deref(), Some("host"));

        headers.insert(COOKIE, HeaderValue::from_static("csrf_token=plain"));
        assert!(extract_csrf_cookie(&headers, true).is_none());
    }

    #[test]
    fn dev_extract_uses_plain_cookie_name() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("csrf_token=plain; __Host-csrf_token=host"),
        );
        assert_eq!(
            extract_csrf_cookie(&headers, false).as_deref(),
            Some("plain")
        );
    }
}
