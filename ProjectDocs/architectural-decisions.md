# Architectural Decision Records

Decisions made before Phase 1 implementation, informed by risk analysis.

---

## ADR-001: Full Repository Trait Pattern for Data Access

**Status**: Accepted
**Date**: 2026-04-08

**Context**: Handlers currently call SurrealDB directly. If we need to move messages to a time-series store or presence to Redis, every handler must change.

**Decision**: Define a trait per entity (`UserRepo`, `MessageRepo`, `ServerRepo`, `ChannelRepo`) with SurrealDB implementations. Handlers receive `Arc<dyn XRepo>` via Axum state.

**Consequences**:
- (+) Can swap storage backends per entity type without touching handlers
- (+) Easy to mock in tests
- (-) More boilerplate upfront
- (-) Trait objects have minor runtime cost (dynamic dispatch)

**Files**: `server/src/repositories/{mod,user,server,channel,message}.rs`

---

## ADR-002: Redis from Day One for Ephemeral State

**Status**: Accepted
**Date**: 2026-04-08

**Context**: Presence, typing indicators, rate limit counters, and refresh tokens are ephemeral and high-frequency. In-memory storage is lost on restart and prevents horizontal scaling.

**Decision**: Add Redis as infrastructure alongside SurrealDB. Use it for:
- Presence state (online/idle/dnd/offline per user)
- Typing indicator TTLs
- Rate limit counters (token bucket)
- Refresh tokens (enables server-side revocation)
- WS auth tickets (single-use, short-lived)

**Consequences**:
- (+) State survives server restarts
- (+) Multiple Axum instances can share ephemeral state (horizontal scaling)
- (+) Redis TTL handles automatic expiry (typing indicators, tickets)
- (-) Additional infrastructure to operate
- (-) Network hop for every presence/rate-limit check

**Dependencies**: `redis` + `deadpool-redis` crates, Redis service in `docker-compose.yml`

---

## ADR-003: WS Protocol + Auth + Rate Limiting Before Business Logic

**Status**: Accepted
**Date**: 2026-04-08

**Context**: These three are the hardest things to change after clients exist. A WS protocol without sequencing can't support reconnection gap-fill. Auth without refresh tokens causes dropped sessions. No rate limiting leaves the system open to abuse.

**Decision**: Implement these as Phase 1 foundation before any handler business logic:
1. WS Protocol v1 — sequence numbers, ACKs, versioning, resume, subscription tiers
2. Auth overhaul — short-lived access tokens (15min), refresh tokens (7d), WS ticket-based auth
3. Rate limiting — Redis-backed token bucket middleware

**Consequences**:
- (+) Protocol is correct from the first line of handler code
- (+) No breaking changes to clients later
- (+) Security posture is solid from the start
- (-) Slower time to first working feature
- (-) More upfront infrastructure work

**Implementation order**: See `phase1-foundation.md`
