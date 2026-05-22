# Auth System Design

## Overview

Replace the current single 24h JWT with a dual-token system and WS ticket-based authentication.

---

## Token Architecture

### Access Token
- **Purpose**: Authenticates REST API requests
- **Lifetime**: 15 minutes
- **Algorithm**: HS256
- **Storage**: Frontend memory (Svelte store), never persisted to localStorage
- **Claims**: `sub` (user ID), `iat`, `exp`

### Refresh Token
- **Purpose**: Obtains new access tokens without re-login
- **Lifetime**: 7 days
- **Storage**: Redis (server-side, keyed by token hash), httpOnly cookie (client-side)
- **Revocation**: Delete from Redis to invalidate instantly
- **Rotation**: Each refresh returns a new refresh token, old one is invalidated

### WS Ticket
- **Purpose**: Authenticates WebSocket connections without exposing JWT in query params
- **Lifetime**: 30 seconds
- **Storage**: Redis with TTL
- **Single-use**: Deleted from Redis after first use
- **Flow**: Client calls `POST /api/auth/ws-ticket` (authenticated) to get a ticket, connects to `ws://host/ws`, then sends `{ "type": "auth", "ticket": "...", "nonce": "..." }` as the first WebSocket message.

---

## Endpoints

### POST /api/auth/register
- **Input**: `{ username, display_name, password }`
- **Validation**: username 3-32 chars alphanumeric, password 8+ chars
- **Returns**: `{ access_token, user }` + sets refresh token cookie
- **Rate limit**: 3 per hour per IP

### POST /api/auth/login
- **Input**: `{ username, password }`
- **Returns**: `{ access_token, user }` + sets refresh token cookie
- **Rate limit**: 10 per minute per IP

### POST /api/auth/refresh
- **Input**: refresh token from httpOnly cookie
- **Validates**: token exists in Redis, not expired
- **Returns**: `{ access_token }` + sets new refresh token cookie
- **Side effect**: Old refresh token deleted from Redis (rotation)

### POST /api/auth/ws-ticket
- **Requires**: Valid access token (Authorization header)
- **Returns**: `{ ticket }` — single-use, 30s TTL
- **Rate limit**: 3 per minute per user

### POST /api/auth/logout
- **Requires**: Valid access token
- **Side effect**: Deletes refresh token from Redis, clears cookie

---

## Frontend Integration

### ApiClient Changes
- On 401 response: automatically call `/api/auth/refresh`
- If refresh succeeds: retry the original request with new access token
- If refresh fails: redirect to login page, clear auth store
- Queue concurrent requests during refresh (don't fire multiple refresh calls)

### Auth Store Changes
- Store access token in memory only (not localStorage)
- `isAuthenticated` derived from token presence + not expired
- On page load: attempt silent refresh via cookie

### WS Client Changes
- Before connecting: call `/api/auth/ws-ticket` to get a ticket
- Connect with `?ticket=<ticket>` instead of `?token=<jwt>`
- On connection drop: get a new ticket before reconnecting

---

## Redis Key Patterns

```
refresh:<token_hash>     → user_id    TTL: 7 days
ws_ticket:<ticket>       → user_id    TTL: 30 seconds
rate:auth:<ip>           → count      TTL: per-window
```

---

## Security Properties

- Access tokens are short-lived (15min) — limits damage window if leaked
- Refresh tokens are server-side revocable — enables forced logout
- Refresh token rotation detects theft — if an old token is reused, all tokens for that user are invalidated
- WS tickets never appear in browser history or server logs (single-use, 30s TTL)
- Passwords hashed with argon2 (via `argon2` crate)
