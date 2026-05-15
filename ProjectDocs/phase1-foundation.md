# Phase 1 Foundation — Implementation Order

Build these layers before any business logic handlers. Each step depends on the previous.

---

## Step 1: Infrastructure Layer

Add Redis to the stack and wire up both database connections.

**Changes**:
- `docker-compose.yml` — add Redis service (port 6379, health check)
- `server/Cargo.toml` — add `redis`, `deadpool-redis`, `argon2`
- `server/src/config.rs` — add `REDIS_URL`, rate limit config values
- `server/src/main.rs` — build `AppState` with SurrealDB + Redis pools
- `.env.example` — add `REDIS_URL=redis://redis:6379`

**New endpoints**:
- `GET /health` — returns 200 if server is running
- `GET /ready` — returns 200 only if SurrealDB + Redis connections are healthy

---

## Step 2: Repository Pattern

Abstract data access behind traits.

**New files**:
- `server/src/repositories/mod.rs` — trait definitions for all repos
- `server/src/repositories/user.rs` — `UserRepo` trait + `SurrealUserRepo`
- `server/src/repositories/server.rs` — `ServerRepo` trait + `SurrealServerRepo`
- `server/src/repositories/channel.rs` — `ChannelRepo` trait + `SurrealChannelRepo`
- `server/src/repositories/message.rs` — `MessageRepo` trait + `SurrealMessageRepo`

**Pattern**:
```rust
#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn create(&self, input: CreateUser) -> Result<User, AppError>;
    async fn find_by_id(&self, id: &RecordId) -> Result<Option<User>, AppError>;
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, AppError>;
    // ...
}
```

Handlers receive repos via Axum state:
```rust
async fn get_user(
    State(repos): State<Arc<Repos>>,
    Path(id): Path<String>,
) -> Result<Json<User>, AppError> {
    // ...
}
```

---

## Step 3: Auth Overhaul

Replace single 24h JWT with dual-token system.

**Modified files**:
- `server/src/auth/jwt.rs` — access token (15min) + refresh token (7d) generation
- `server/src/auth/middleware.rs` — updated extractor, refresh token validation
- `server/src/handlers/users.rs` — register/login return token pair, add refresh + ws-ticket + logout endpoints
- `client/src/lib/api/client.ts` — auto-refresh on 401, request queuing
- `client/src/lib/stores/auth.ts` — memory-only token, silent refresh on load
- `client/src/lib/ws/client.ts` — ticket-based auth flow

See `auth-design.md` for full specification.

---

## Step 4: WS Protocol v1

Redesign the WebSocket message format.

**Modified files**:
- `server/src/ws/connection.rs` — new message parsing with version, sequence, ACK, resume, subscribe/unsubscribe
- `server/src/ws/room.rs` — subscription tiers (active vs badge), targeted delivery, sequence number tracking
- `server/src/ws/presence.rs` — Redis-backed (replaces in-memory HashMap)
- `client/src/lib/ws/client.ts` — new protocol handling: auth flow, resume, ACK, heartbeat
- `client/src/lib/stores/chat.ts` — per-channel message buffers, optimistic updates with pending/sent/failed states

See `ws-protocol-v1.md` for full specification.

---

## Step 5: Rate Limiting

Redis-backed token bucket middleware.

**New files**:
- `server/src/middleware/mod.rs`
- `server/src/middleware/rate_limit.rs`

**Rate limit categories**:

| Category | Limit | Window | Scope |
|----------|-------|--------|-------|
| `message_send` | 5 | 5 seconds | per user per channel |
| `api_general` | 30 | 1 minute | per user |
| `ws_connect` | 3 | 1 minute | per user |
| `friend_request` | 10 | 1 hour | per user |
| `auth_login` | 10 | 1 minute | per IP |
| `auth_register` | 3 | 1 hour | per IP |

Returns `429 Too Many Requests` with `Retry-After` header.

---

## Step 6: Observability Basics

**Changes**:
- `server/src/main.rs` — structured JSON logging, request ID middleware
- WS handlers — propagate request IDs into handler context
- Add tracing spans for handler execution timing

**Metrics to expose** (via `/metrics` or logs):
- Connected WS clients (gauge)
- Messages per second (counter)
- Active room actors (gauge)
- DB query latency (histogram)
- Rate limit rejections (counter)

---

## Verification Checklist

After all 6 steps are complete:

- [ ] `docker compose up` starts SurrealDB + Redis + server + client
- [ ] `GET /health` returns 200
- [ ] `GET /ready` returns 200 (both DB and Redis healthy)
- [ ] Repository traits compile with SurrealDB implementations
- [ ] Register returns access token + sets refresh cookie
- [ ] Access token expires in 15min, refresh flow returns new pair
- [ ] Refresh token rotation invalidates the old token
- [ ] `POST /api/auth/ws-ticket` returns single-use ticket
- [ ] WS connects via ticket, receives `auth_ok`
- [ ] WS messages include `v: 1` and `seq` fields
- [ ] Send message → receive ACK with server-assigned ID
- [ ] Disconnect and reconnect with `resume` → missed messages delivered
- [ ] Subscribe with `level: "badge"` → receive only unread counts
- [ ] Rate limit returns 429 after threshold exceeded
- [ ] Presence state in Redis survives server restart
- [ ] Structured logs include request IDs
