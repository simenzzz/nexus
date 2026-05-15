# Architectural Risk Analysis

Identified before implementation, drawing from Discord, Slack, Netflix, and Jira at scale.

---

## 1. SurrealDB as Single Database

**Risk**: Chat messages (append-heavy, time-ordered) and graph queries (read-heavy, relational) have fundamentally different access patterns. Mixing them on one engine means a burst of chat activity degrades graph query performance.

**How others solve it**:
- Discord: PostgreSQL (metadata) + ScyllaDB (messages) + Redis (ephemeral state) — three systems optimized per access pattern
- Slack: MySQL sharded via Vitess (messages) + Redis (sessions/cache) + Elasticsearch (search) + S3 (cold storage)

**Mitigation**: Full repository trait pattern isolates handlers from SurrealDB query syntax. If we hit performance walls, we can swap storage backends per entity type without rewriting business logic.

**Additional concern — Search**: SurrealDB has full-text search, but it's not a dedicated search engine. Searching millions of messages by content, author, channel, and date range will eventually need Elasticsearch or similar.

---

## 2. Fan-Out Problem (Presence & Message Delivery)

**Risk**: Sending a message to a 1,000-member channel = 1,000 WebSocket writes. A user coming online = N writes to every friend. Discord found this cost grows **quadratically** with guild size (more members = more activity x more recipients).

**How Discord solved it**:
- 90% of large guild users are passive — don't send them real-time presence updates
- "Lazy guilds" — clients request only the visible portion of member lists
- Relay system — for guilds > 15,000 members, fan-out splits across processes each handling ~15,000 users
- Presence is scoped — only propagated to users sharing a server or friendship

**Mitigation**: Design subscription tiers from day one:
- **Active**: full messages + typing indicators (user is viewing the channel)
- **Badge**: unread count only (user is a member but viewing another channel)
- **None**: no events (not in server)

---

## 3. WebSocket Protocol Gaps

**Risk**: The WS message format is a protocol. Once clients depend on it, changing it is painful. Current scaffolding has no sequencing, no ACKs, no versioning, and no gap-fill mechanism.

**How Slack got burned**: RTM Start method assembled a giant JSON payload on every connection. For 10,000+ person orgs, this ballooned to tens of megabytes. They had to completely re-architect.

**Mitigation**: Design WS Protocol v1 with sequence numbers, ACKs, versioning, and resume before writing any handlers. See `ws-protocol-v1.md`.

---

## 4. Session Initialization (The Slack Trap)

**Risk**: When a user opens the app, sending all channels, members, unread counts, and presence in one payload doesn't scale. Slack learned this the hard way.

**Mitigation**:
- Lazy loading from the start — on app open, fetch only: server list with badges, active channel's recent messages, presence for visible friends
- Paginate everything — never send "all X" in one response
- Precompute badges server-side — store `last_read_seq` per user per channel, compute deltas on the server

---

## 5. CRDT Integration Challenges (Phase 2)

**Risk areas**:
- **Tombstone accumulation**: Yjs's YATA protocol GCs tombstones after ~30s, but can't safely GC while peers might be disconnected. Documents grow without bound.
- **Rich text is an open research problem**: Yjs handles it via XML/ProseMirror mapping, but edge cases (bold across deletion boundaries, nested lists) produce unintuitive merges. The Moment.dev team abandoned Yjs for rich text.
- **Backend bottleneck**: Every keystroke from every collaborator hits the server when backend is the authoritative peer. Needs batching/debouncing.
- **Memory pressure**: Each active Y.Doc in server RAM. 100 concurrent editing sessions = 100 Y.Docs growing in memory.

**Mitigation**: Start with plain text CRDTs. Add rich text later. Debounce updates (1-5s intervals). Set document size limits. Store periodic snapshots for new peer bootstrap.

---

## 6. Whiteboard Scaling (Phase 3)

**Risk areas**:
- Canvas operations are larger than text edits (freehand paths = hundreds of points)
- Z-ordering conflicts — two users moving objects to "front" simultaneously produces merged results that match neither intent
- Redrawing thousands of shapes on every CRDT update kills canvas performance

**Mitigation**: Stream stroke data incrementally (not full path on mouse-up). Decouple CRDT state from render state (diff-based redraws). Compress freehand paths (Ramer-Douglas-Peucker) before storing.

---

## 7. Watch-Together Clock Sync (Phase 4)

**Risk**: Client A has 50ms latency, Client B has 200ms. Without compensation, they'll be noticeably out of sync (humans notice >100ms audio desync).

**How Netflix solves it**: NTP-style time sync, periodic sync pulses with timestamp + playback rate, clients adjust playback rate (1.01x/0.99x) to gradually converge rather than jumping.

**Mitigation**: Implement latency estimation (ping/pong EMA). Use gradual sync (speed up/slow down) not jump sync. Define tolerance window (~500ms).

---

## 8. Auth & Security Gaps

**Risk areas**:
- 24h JWT with no refresh flow — token expires mid-session, WS drops, state lost
- WS token in query params — ends up in server logs, proxy logs, browser history
- No rate limiting — one bad actor can flood the system

**Mitigation**: See `auth-design.md` for the full overhaul. Short-lived access tokens (15min) + refresh tokens. WS ticket-based auth. Rate limiting middleware from day one.

---

## 9. Frontend State Management

**Risk**: Current `chat` store holds a flat `messages` array. Loading all messages for all channels = unbounded memory. No cache invalidation, no optimistic updates.

**Mitigation**:
- Per-channel message buffers with cap (~200 messages), older evicted
- Optimistic message insertion: pending → sent (on ACK) → failed (on timeout)
- Store normalization: separate stores keyed by ID (servers, channels, members, messages, presence)

---

## 10. Observability

**Risk**: No metrics, no structured logging, no tracing beyond basic `tracing-subscriber`. At scale, you're flying blind.

**Mitigation**: Structured JSON logging with request IDs propagating through WS handlers. Metrics: connected clients, messages/sec, room count, DB latency. Health endpoints for load balancers.

---

## Sources

- [How Discord Scaled to 15 Million Users on One Server](https://www.geeksforgeeks.org/system-design/how-discord-scaled-to-15-million-users-on-one-server/)
- [Discord: Real-Time Architecture at Internet Scale](https://medium.com/@yadavmpadiyar/%EF%B8%8F-scaling-up-5-discord-real-time-architecture-at-internet-scale-bef4be6b7198)
- [Maxjourney: Pushing Discord's Limits with a Million+ Online Users](https://discord.com/blog/maxjourney-pushing-discords-limits-with-a-million-plus-online-users-in-a-single-server)
- [Real-time Communication at Scale with Elixir at Discord](https://elixir-lang.org/blog/2020/10/08/real-time-communication-at-scale-with-elixir-at-discord/)
- [Slack Architecture — System Design](https://systemdesign.one/slack-architecture/)
- [Changing the Model: Why and How Slack Re-Architected](https://www.infoq.com/presentations/slack-rearchitecture/)
- [SurrealDB 3.0 Benchmarks](https://surrealdb.com/blog/surrealdb-3-0-benchmarks-a-new-foundation-for-performance)
- [Lies I Was Told About Collaborative Editing (Moment.dev)](https://www.moment.dev/blog/lies-i-was-told-pt-2)
- [WebSockets at Scale: Architecture for Millions of Connections](https://websocket.org/guides/websockets-at-scale/)
