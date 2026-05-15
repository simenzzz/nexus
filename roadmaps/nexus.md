# Nexus — Complete Roadmap

A social media / Discord hybrid focused on **graph data modeling** (SurrealDB), **real-time collaboration** (WebSockets + CRDTs), and **production-grade infrastructure**.

## Tech Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Backend | Rust + Axum + Tokio | High-performance async, great for concurrent WS connections |
| Database | SurrealDB | Native graph edges + document storage, first-class Rust SDK |
| Cache/Ephemeral | Redis | Presence, rate limits, refresh tokens, WS tickets |
| CRDT Engine | Yrs (Yjs in Rust) | Battle-tested CRDTs for real-time collaborative editing |
| Frontend | SvelteKit + TypeScript | Lightweight, reactive, good WS/canvas ergonomics |
| Auth | JWT dual-token | Short-lived access (15min) + refresh (7d) + WS tickets |
| Real-time | WebSockets via Axum | Protocol v1 with seq numbers, ACKs, resume |

## Architecture

```
┌─────────────────────────────────────────────────┐
│                 SvelteKit Frontend               │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │ Chat UI  │  │ Feed/Post│  │ Canvas/Player │  │
│  └────┬─────┘  └────┬─────┘  └──────┬────────┘  │
│       │WS            │REST+WS        │WS (CRDT)  │
└───────┼──────────────┼───────────────┼───────────┘
        │              │               │
┌───────┴──────────────┴───────────────┴───────────┐
│                  Axum Backend                     │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │ Chat     │  │ Social   │  │ Collab Engine │  │
│  │ Rooms    │  │ Graph    │  │ (Yrs CRDTs)  │  │
│  │ (actors) │  │ Queries  │  │              │  │
│  └────┬─────┘  └────┬─────┘  └──────┬────────┘  │
│       │              │               │            │
│  ┌────┴──────────────┴───────────────┴─────────┐ │
│  │         SurrealDB         Redis              │ │
│  │  (documents + graph)  (ephemeral state)      │ │
│  └──────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────┘
```

---

## Phase 0: Foundation Infrastructure

**Status**: Not started
**Goal**: Production-grade plumbing before any business logic. These are the hardest things to change later.
**Depends on**: Scaffolding complete (done)

### 0.1 Infrastructure Layer

Add Redis to the stack and wire up database connections.

- Add Redis service to `docker-compose.yml` (port 6379, health check)
- Add `redis`, `deadpool-redis`, `argon2` to `Cargo.toml`
- Add `REDIS_URL` to config and `.env.example`
- Build `AppState` with SurrealDB + Redis connection pools
- Health endpoints: `GET /health` (server running), `GET /ready` (DB + Redis healthy)

### 0.2 Repository Pattern

Abstract data access behind traits so handlers don't depend on SurrealDB directly.

**New files**: `server/src/repositories/{mod,user,server,channel,message}.rs`

Pattern:
```rust
#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn create(&self, input: CreateUser) -> Result<User, AppError>;
    async fn find_by_id(&self, id: &RecordId) -> Result<Option<User>, AppError>;
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, AppError>;
}
```

Handlers receive `Arc<dyn XRepo>` via Axum state, not raw DB handles.

### 0.3 Auth Overhaul

Replace single 24h JWT with dual-token system. Full spec in `ProjectDocs/auth-design.md`.

- **Access token**: 15min, HS256, memory-only on client
- **Refresh token**: 7d, stored in Redis (server-side revocable), httpOnly cookie
- **WS ticket**: 30s, single-use, Redis — replaces raw JWT in query params
- **Endpoints**: register, login, refresh, ws-ticket, logout
- **Frontend**: auto-refresh on 401, silent refresh on page load

### 0.4 WebSocket Protocol v1

Full spec in `ProjectDocs/ws-protocol-v1.md`.

- Every message gets `"v": 1` (protocol version)
- Server-assigned monotonic sequence number per channel (`seq`)
- Message ACK: sender gets `message_ack` with server-assigned ID + seq
- Resume protocol: client sends `last_seq` per channel, server replays or sends `resync`
- Subscription tiers: `active` (full events) vs `badge` (unread count only)
- Heartbeat: client sends every 30s, server detects idle (5min) and disconnect (60s)

### 0.5 Rate Limiting

Redis-backed token bucket middleware.

| Category | Limit | Window | Scope |
|----------|-------|--------|-------|
| `message_send` | 5 | 5 seconds | per user per channel |
| `api_general` | 30 | 1 minute | per user |
| `ws_connect` | 3 | 1 minute | per user |
| `friend_request` | 10 | 1 hour | per user |
| `auth_login` | 10 | 1 minute | per IP |
| `auth_register` | 3 | 1 hour | per IP |

### 0.6 Observability Basics

- Structured JSON logging with request IDs
- Request ID propagation into WS handler context
- Tracing spans for handler execution timing
- Metrics: connected clients, messages/sec, room count, DB latency, rate limit rejections

### 0.x Verification Checklist

- [ ] `docker compose up` starts SurrealDB + Redis + server + client
- [ ] `GET /health` returns 200, `GET /ready` returns 200
- [ ] Repository traits compile with SurrealDB implementations
- [ ] Register returns access token + sets refresh cookie
- [ ] Access token expires in 15min, refresh flow works
- [ ] WS connects via ticket, receives `auth_ok`
- [ ] WS messages include `v: 1` and `seq` fields
- [ ] Send message → receive ACK with server ID
- [ ] Disconnect + reconnect with `resume` → missed messages delivered
- [ ] Rate limit returns 429 after threshold
- [ ] Presence survives server restart (in Redis)

---

## Phase 1: Social Graph + Real-time Chat

**Status**: Not started
**Goal**: Graph data model, WebSocket chat, presence, social features.
**Depends on**: Phase 0 complete

### 1.1 Graph Data Model (SurrealDB)

**Records:**
- `user` — id, username, display_name, avatar_url, status, created_at
- `server` — id, name, description, icon_url, owner (user ref), created_at
- `channel` — id, name, channel_type (text | voice | collab), server (ref), created_at
- `message` — id, content, author (user ref), channel (ref), created_at, edited_at

**Graph Edges:**
- `follows` (user → user) — one-directional social follow
- `friends_with` (user ↔ user) — mutual friendship (two directed edges)
- `member_of` (user → server) — metadata: role, joined_at
- `blocked` (user → user) — block relationship

**Key Graph Queries:**
- Mutual friends: `SELECT ->friends_with->user FROM $user WHERE ->friends_with->user->friends_with CONTAINS $other`
- Friend suggestions: friends-of-friends not already connected (2-hop traversal)
- Server discovery: servers your friends are in, ranked by member overlap
- Feed ranking: posts weighted by graph distance from viewer

### 1.2 Real-time Chat

- WebSocket upgrade endpoint at `/ws` (authenticated via WS ticket)
- **Room actor pattern**: each channel is a Tokio task
  - Holds `HashMap<UserId, Sender>` of connected clients
  - Receives messages via `mpsc` channel
  - Broadcasts to active subscribers, sends badge updates to badge subscribers
- Message types: `ChatMessage`, `Typing`, `Join`, `Leave`, `Presence`
- Message history via REST `GET /channels/{id}/messages?before=&limit=`
- Typing indicators with 3-second debounce/timeout
- Messages indexed by `(channel_id, created_at)` for efficient pagination

### 1.3 Presence System

- Track online/idle/DND/offline per user in Redis
- On WS connect: set online, notify friends via graph traversal
- On WS disconnect: set offline after 30s grace period
- Idle detection: client sends heartbeat, server marks idle after 5min silence
- Presence only propagated to users who share a server or friendship edge
- Diff-based updates — send only changes, not full presence map

### 1.4 Frontend Integration

- Lazy loading: on app open, fetch only server list + active channel messages + visible friend presence
- Per-channel message buffers (max ~200), older messages evicted, fetched via REST on scroll-up
- Optimistic message insertion: pending → sent (on ACK) → failed (on timeout)
- Normalized stores: separate stores keyed by ID (servers, channels, members, messages, presence)
- Precomputed unread badges: `last_read_seq` per user per channel, server computes delta

### 1.5 Verification Checklist

- [ ] Create users, establish friendships → graph edges exist in SurrealDB
- [ ] Mutual friends query returns correct results
- [ ] Friend suggestions exclude existing friends
- [ ] Server discovery returns servers ranked by friend overlap
- [ ] Two browser tabs can chat in real-time in the same channel
- [ ] Typing indicator appears and disappears with debounce
- [ ] Presence updates propagate to friends but not strangers
- [ ] Message history loads correctly with pagination
- [ ] Optimistic messages show pending state, then resolve to sent
- [ ] Reconnect after disconnect: missed messages delivered via resume

---

## Phase 2: Collaborative Posts (CRDT)

**Status**: Not started
**Goal**: Google Docs-style co-editing for server posts using CRDTs.
**Depends on**: Phase 1 complete

### 2.1 CRDT Integration

- Integrate `yrs` crate into the backend
- **Start with plain text** — rich text CRDTs are an open research problem (defer to later)
- Each collaborative post is a `Y.Doc` instance
- Sync protocol: Yjs sync v1 (state vectors + update deltas) over WebSocket
- Backend acts as authoritative CRDT peer — merges all updates, persists state
- On client connect: send full state vector, then incremental updates
- Debounce persistence: batch updates on 1-5 second intervals, not every keystroke

### 2.2 Post Lifecycle

1. **Create draft** → spawns a Y.Doc, opens a collab WS room
2. **Invite collaborators** → graph determines eligible users (friends or server members)
3. **Co-edit** → all connected users see real-time changes and cursors
4. **Publish** → freeze Y.Doc, extract final content, store as immutable `post` record
5. **View** → published posts display in server feed, ranked by graph distance

### 2.3 Awareness Protocol

- Cursor positions broadcast to all collaborators
- Selection highlights in each collaborator's assigned color
- User list showing who's currently editing
- Idle/active status within the editing session

### 2.4 Document Management

- Periodic snapshots stored in SurrealDB (new peers bootstrap from snapshot, not full history)
- Max document size limit (prevent unbounded CRDT growth from tombstones)
- Max collaborators per document
- Y.Doc eviction from memory when no active editors (rehydrate on reconnect)

### 2.5 New Files

**Backend:**
- `src/collab/mod.rs` — CRDT document manager (HashMap of active Y.Docs)
- `src/collab/doc.rs` — Y.Doc lifecycle: create, apply update, encode state, persist
- `src/collab/awareness.rs` — cursor/selection state broadcasting
- `src/handlers/posts.rs` — post CRUD + publish endpoint

**Frontend:**
- `CollabEditor.svelte` — text editor with yjs bindings
- `CollaboratorCursors.svelte` — overlay showing other users' cursors
- `CollabInvite.svelte` — invite friends/members to co-edit
- `PostCard.svelte` — published post display in feed

### 2.6 Verification Checklist

- [ ] Create a draft post → Y.Doc initialized on server
- [ ] Two users open the same draft → edits sync in real-time
- [ ] Cursor positions visible for all collaborators
- [ ] Publish post → content frozen, collab room closed
- [ ] Published post appears in server feed
- [ ] Disconnect and reconnect → state catches up via sync protocol
- [ ] Only eligible users (friends/members) can be invited
- [ ] Y.Doc evicted from memory after all editors leave, rehydrated on return

---

## Phase 3: Shared Whiteboard

**Status**: Not started
**Goal**: Collaborative canvas drawing via CRDTs.
**Depends on**: Phase 2 complete (reuses CRDT infrastructure)

### 3.1 Whiteboard as Channel Type

- New channel type: `whiteboard`
- Each whiteboard channel gets a persistent Y.Doc
- Drawing operations stored as items in a `Y.Array` (ordered draw commands)
- Each item: `{ type, points, color, width, layer, z_index, author }`

### 3.2 Drawing Tools

- Freehand pen (path as array of points)
- Shapes: rectangle, circle, line, arrow
- Text labels
- Eraser (marks items as deleted in CRDT)
- Select + move (updates position fields)
- Color picker, stroke width

### 3.3 Performance Considerations

- Stream stroke data incrementally (every N ms), not full path on mouse-up
- Decouple CRDT state from render state — diff-based redraws, not full canvas repaints
- Compress freehand paths (Ramer-Douglas-Peucker) before storing in CRDT
- Z-ordering conflicts: CRDTs resolve deterministically, but UX should surface conflicts

### 3.4 Layers and Z-Ordering

- `Y.Array` per layer
- Z-ordering via array position
- Layer visibility toggles (client-side only)
- Lock layers to prevent editing

### 3.5 Persistence

- Y.Doc state encoded and stored in SurrealDB on interval (every 30s) and on last-user-disconnect
- On first user connect: load from DB, hydrate Y.Doc
- History: store periodic snapshots for undo-to-checkpoint

### 3.6 New Files

**Backend:**
- `src/collab/whiteboard.rs` — whiteboard-specific CRDT logic, shape types

**Frontend:**
- `Whiteboard.svelte` — main canvas component (HTML5 Canvas)
- `DrawingTools.svelte` — toolbar for pen/shapes/eraser
- `WhiteboardLayer.svelte` — layer panel
- `WhiteboardCursors.svelte` — show other users' cursor positions on canvas

### 3.7 Verification Checklist

- [ ] Draw on whiteboard in tab A → appears in tab B in real-time
- [ ] Multiple simultaneous drawers produce correct merged result
- [ ] Reload page → whiteboard state loads from DB
- [ ] Eraser removes strokes for all users
- [ ] Select and move a shape → position updates for everyone
- [ ] Layer ordering works correctly
- [ ] Freehand paths are compressed before storage

---

## Phase 4: Watch-Together Rooms

**Status**: Not started
**Goal**: Synchronized shared media experience with graph-powered recommendations.
**Depends on**: Phase 1 complete (can run parallel to Phases 2-3)

### 4.1 Watch Room as Channel Type

- New channel type: `watch`
- Room state: current media URL, playback position, playing/paused, queue
- State synced via WebSocket (leader-based sync, not CRDT)

### 4.2 Playback Sync

- **Leader model**: room owner or designated leader controls playback
- Leader actions (play, pause, seek) broadcast to all members
- Latency estimation via ping/pong EMA — offset playback timestamps per client
- Gradual sync: adjust playback rate (1.01x/0.99x) to converge, not jump
- Tolerance window: don't correct drifts under 500ms
- Periodic sync pulses every 5 seconds to correct accumulated drift

### 4.3 Queue and Voting

- Members can add media to the queue
- Upvote/downvote items → queue re-sorted by score
- Auto-advance to next item when current finishes
- Graph edge: `queued` (user → media in room)

### 4.4 Live Reactions

- Emoji reactions float up on screen (Twitch-style)
- Broadcast via WS to all room members
- Rate-limited to prevent spam (5 per second per user)

### 4.5 Graph-Based Recommendations

- New graph edge: `watched` (user → media) with metadata: watch_count, last_watched
- Recommendation query: traverse user → servers → members → watched → media
- Filter out already-watched, rank by frequency across the subgraph
- Display as "Suggested for this room" based on collective taste

### 4.6 New Files

**Backend:**
- `src/ws/watch_room.rs` — synced playback room actor
- `src/graph/recommendations.rs` — graph traversal for media suggestions
- `src/handlers/watch.rs` — queue management endpoints

**Frontend:**
- `WatchRoom.svelte` — main watch-together view
- `PlaybackControls.svelte` — play/pause/seek + sync indicator
- `VoteQueue.svelte` — queue list with voting
- `ReactionOverlay.svelte` — floating emoji reactions
- `Recommendations.svelte` — suggested media panel

### 4.7 Verification Checklist

- [ ] Leader plays/pauses → all members' playback updates
- [ ] Seek to timestamp → all members jump to correct position (with latency compensation)
- [ ] Add item to queue → appears for all members
- [ ] Vote on queue item → order updates in real-time
- [ ] Emoji reactions visible for all room members
- [ ] Recommendations improve as more watch history accumulates
- [ ] New member joining mid-playback syncs to correct position

---

## Milestone Summary

| Phase | Core Learning | Key Tech | Depends On |
|-------|--------------|----------|------------|
| 0 | Production infrastructure | Redis, repository traits, WS protocol, auth | Scaffolding |
| 1 | Graph modeling, WebSocket fundamentals | SurrealDB graphs, Axum WS, Tokio actors | Phase 0 |
| 2 | CRDTs, collaborative editing | Yrs, TipTap, Yjs sync protocol | Phase 1 |
| 3 | Complex state sync, canvas | Yrs + Canvas API | Phase 2 |
| 4 | Graph algorithms, media sync | Graph traversals, leader-based sync | Phase 1 |

## Key References

- `ProjectDocs/architectural-risks.md` — 10 risk areas with industry examples
- `ProjectDocs/architectural-decisions.md` — ADRs for repository pattern, Redis, foundation-first
- `ProjectDocs/ws-protocol-v1.md` — Full WS protocol specification
- `ProjectDocs/auth-design.md` — Dual-token auth system design
- `ProjectDocs/phase1-foundation.md` — Detailed Phase 0 implementation steps
