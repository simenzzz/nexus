# Hardening Notes

This document captures security-relevant changes shipped in the final
review pass and items deferred for follow-up. The main `CLAUDE.md` and
`ProjectDocs/` describe steady-state behaviour; this file is the audit
trail for what was tightened, why, and what remains.

## Shipped in the final review pass

### Config + secret hygiene
- New `NEXUS_ENV` flag (`development` | `production`). In production the
  server refuses to start unless:
  - `JWT_SECRET` is ≥ 32 chars and not the example placeholder
  - `SURREAL_USER` is not `root` and `SURREAL_PASS` is not `root` / ≥ 16 chars
  - `SECURE_COOKIES=true`
  - `CORS_ORIGIN` is explicitly set (no localhost fallback)
- JWT test secret moved behind `cfg(test)` — it cannot reach a release build.
- Connection URL no longer logged at startup; only ns/db are.

### HTTP surface
- `security_headers` middleware applied to every response:
  `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`,
  `Permissions-Policy`, `Content-Security-Policy`.
- `DefaultBodyLimit::max(1 MB)` applied globally.
- CORS `Access-Control-Max-Age: 3600`.
- Request-ID middleware moved to top-level so health routes also emit
  `x-request-id` for log correlation.

### Auth
- WS ticket TTL reduced 30s → 10s.
- WS ticket binds a per-issue **nonce** (16 bytes); the client must echo it
  on the `/ws` query string. Stored as **two sibling Redis keys** consumed
  atomically via MULTI/EXEC GETDEL, so neither value contaminates the
  other on weird inputs. Constant-time nonce comparison.
- `/api/auth/refresh` and `/api/auth/ws-ticket` are per-user rate-limited
  (1/5s and 1/2s respectively). Each rejection logs a structured
  `auth_rate_limited` event.
- Double-submit CSRF on `/api/auth/refresh` and `/api/auth/logout`:
  - Server sets a non-HttpOnly `csrf_token` cookie on login + refresh.
  - Client echoes it via `X-CSRF-Token` on subsequent state-changing calls.
  - Constant-time comparison on the server.
- Both `csrf_token` and `refresh_token` cookies use the `__Host-` prefix
  in production (Secure + Path=/ + no Domain) to block parent-domain
  cookie planting. Dev mode keeps the plain name for non-Secure cookies.

### WS input
- Watch queue title rejected at the connection boundary (`TITLE_TOO_LONG`)
  instead of silently truncated downstream.
- Awareness rate limit tightened from 30/s to 2/s on both whiteboard and
  post awareness paths (DoS amplification ceiling).
- Heartbeat enforces a server-side minimum interval of 2s per connection.

### Error responses
- All `Database` / `Redis` / `Internal` errors return a fixed `"internal
  error"` message — no DB string leaks. Operators correlate via the
  `x-request-id` response header.

## Module size

Three modules previously violated the 800-line rule. After this pass:

| File                                  | Before | After | Notes |
|---------------------------------------|-------:|------:|-------|
| `server/src/ws/watch_room.rs`         |   886  |   763 | Under budget. Helpers extracted to `ws/watch_room_helpers.rs`. |
| `server/src/collab/mod.rs`            |  1025  |   894 | Production code is **362 lines**; the rest is the co-located test module. Task loops moved to `collab/tasks.rs`. |
| `server/src/ws/connection.rs`         |  1184  |  1042 | Helpers moved to `ws/connection_helpers.rs`. **Still over budget — see follow-up.** |

### Follow-up: shrink `connection.rs`
The remaining oversize is concentrated in the giant message-dispatch match.
A clean split needs a `ConnectionContext` struct carrying the per-session
mutable state (`subscriptions`, `watch_subscriptions`, `last_typing`,
`audience`, `is_idle`, etc.) so the match arms can become one-line calls
into per-domain handler files (`chat.rs`, `watch.rs`, `whiteboard.rs`).
Tracked as the next refactor.

### Infrastructure
- Redis now requires AUTH (`--requirepass`); `REDIS_URL` carries the
  password in the userinfo segment.
- Compose interpolation uses `${VAR:?…}` for `SURREAL_USER`, `SURREAL_PASS`,
  and `REDIS_PASSWORD` so a missing-value deploy fails fast — no more
  `:-root` fallbacks.
- Caddy `request_body max_size` aligned to the server's 1 MB
  `DefaultBodyLimit` (no asymmetric accept/reject window).
- Middleware order reorganized so `request_id` is outermost — `TraceLayer`
  spans include the correlation id.

## Review findings addressed

The mandatory `code-reviewer` and `security-reviewer` passes flagged the
following items, all addressed before this branch was declared done:
- HIGH: misleading "Docker secrets" comment in prod compose — replaced
  with honest description of `env_file` model + pointer to how to switch
  to real secrets.
- HIGH: env-mutating config test was unserialized — wrapped in a
  `static ENV_LOCK: Mutex<()>` and `dotenvy::dotenv()` moved out of
  `from_env()` into `main.rs` so tests aren't repopulated mid-run.
- HIGH/MEDIUM: `__Host-` cookie prefix for production CSRF + refresh.
- MEDIUM: middleware ordering swap (`request_id` outermost).
- MEDIUM: WS ticket separator collision replaced with two-key MULTI/EXEC.
- MEDIUM: Caddy body limit aligned to server.
- MEDIUM: client retries CSRF after silent refresh.
- MEDIUM: SurrealDB title byte vs char metric divergence — DB assert
  raised to 800 bytes (4× the connection-boundary char cap) so emoji-heavy
  titles passing the boundary aren't rejected later by the schema.
- MEDIUM: Redis AUTH + drop of `:-root` defaults in compose.

## Deferred (LOW)

These were rated LOW by the audit and intentionally not addressed in this
pass. Each is a small, isolated change.

1. **Refresh-token TTL** — currently 7d, could be reduced to 3d (config
   change in `.env`). Lower blast radius if a refresh token leaks.
2. **Cross-tab logout** — `BroadcastChannel` on `/api/auth/logout` so
   secondary tabs flush their in-memory access tokens. Pure client work.
3. **Request-ID in body** — request_id is already in the response header
   (`x-request-id`). Echoing it into the JSON error body too would let
   strict clients show the operator a copy/paste handle, but the header
   is sufficient for log correlation.
4. **`/api/auth/me` invalidation on profile changes** — out of scope for
   this pass; once profile edit endpoints land, they should bump a version
   cookie or revoke the access token.

## Verification

Smoke-test commands captured in `/.claude/plans/the-entire-project-is-glittery-deer.md`
under "Phase E — Verification."
