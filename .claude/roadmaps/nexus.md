# Nexus — Social Media / Discord Hybrid

A learning project focused on **graph data modeling** (SurrealDB) and **real-time collaboration** (WebSockets + CRDTs).

## Tech Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Backend | Rust + Axum + Tokio | High-performance async, great for concurrent WS connections |
| Database | SurrealDB | Native graph edges + document storage, first-class Rust SDK |
| CRDT Engine | Yrs (Yjs in Rust) | Battle-tested CRDTs for real-time collaborative editing |
| Frontend | SvelteKit + TypeScript | Lightweight, reactive, good WS/canvas ergonomics |
| Auth | JWT + SurrealDB scopes | Built-in auth scopes in SurrealDB |
| Real-time | WebSockets via Axum | Native Axum support, Tokio tasks as room actors |

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
│  │              SurrealDB                       │ │
│  │  Users ──follows──▶ Users                    │ │
│  │  Users ──member_of──▶ Servers                │ │
│  │  Messages, Posts, Channels (documents)       │ │
│  └──────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────┘
```

---

## Phase 1: Social Graph + Real-time Chat

**Status**: Not started
**Goal**: Foundation — graph data model, WebSocket fundamentals, core app structure.

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

- WebSocket upgrade endpoint at `/ws` (authenticated via JWT)
- **Room actor pattern**: each channel is a Tokio task
  - Holds `HashSet<UserId>` of connected clients
  - Receives messages via `mpsc` channel
  - Broadcasts to all connected clients via their `mpsc` senders
- Message types: `ChatMessage`, `Typing`, `Join`, `Leave`, `Presence`
- Message history via REST `GET /channels/{id}/messages?before=&limit=`
- Typing indicators with 3-second debounce/timeout

### 1.3 Presence System

- Track online/idle/DND/offline per user
- On WS connect: set online, notify friends via graph traversal
- On WS disconnect: set offline after 30s grace period
- Idle detection: client sends heartbeat, server marks idle after 5min silence
- Presence only propagated to users who share a server or friendship edge

### 1.4 Verification Checklist

- [ ] Create users, establish friendships → graph edges exist in SurrealDB
- [ ] Mutual friends query returns correct results
- [ ] Friend suggestions exclude existing friends
- [ ] Server discovery returns servers ranked by friend overlap
- [ ] Two browser tabs can chat in real-time in the same channel
- [ ] Typing indicator appears and disappears with debounce
- [ ] Presence updates propagate to friends but not strangers
- [ ] Message history loads correctly with pagination

### 1.5 Backend Structure

```
server/
├── Cargo.toml
├── src/
│   ├── main.rs                # Axum app setup, route registration
│   ├── config.rs              # Env vars, DB connection config
│   ├── error.rs               # Unified error type (thiserror)
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── jwt.rs             # Token creation/validation
│   │   └── middleware.rs      # Axum auth extractor
│   ├── models/
│   │   ├── mod.rs
│   │   ├── user.rs
│   │   ├── server.rs
│   │   ├── channel.rs
│   │   └── message.rs
│   ├── graph/
│   │   ├── mod.rs
│   │   ├── social.rs          # Friends, mutual friends, suggestions
│   │   ├── discovery.rs       # Server recommendations
│   │   └── feed.rs            # Feed ranking by graph distance
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── users.rs           # CRUD + profile
│   │   ├── servers.rs         # CRUD + membership
│   │   ├── channels.rs        # CRUD
│   │   └── messages.rs        # History retrieval
│   └── ws/
│       ├── mod.rs
│       ├── connection.rs      # Per-user WS connection handler
│       ├── room.rs            # Channel room actor (Tokio task)
│       └── presence.rs        # Online status tracking + propagation
```

### 1.6 Frontend Structure

```
client/
├── package.json
├── svelte.config.js
├── src/
│   ├── routes/
│   │   ├── +layout.svelte          # App shell
│   │   ├── (auth)/
│   │   │   ├── login/+page.svelte
│   │   │   └── register/+page.svelte
│   │   └── (app)/
│   │       ├── +layout.svelte      # Authenticated layout (sidebar + main)
│   │       ├── feed/+page.svelte   # Social feed
│   │       ├── servers/
│   │       │   └── [serverId]/
│   │       │       └── channels/
│   │       │           └── [channelId]/+page.svelte
│   │       ├── friends/+page.svelte
│   │       └── explore/+page.svelte # Server discovery
│   └── lib/
│       ├── stores/
│       │   ├── auth.ts
│       │   ├── chat.ts             # Messages, active room
│       │   └── presence.ts         # Online status store
│       ├── ws/
│       │   └── client.ts           # WebSocket manager (connect, reconnect, parse)
│       ├── api/
│       │   └── client.ts           # REST API wrapper
│       └── components/
│           ├── MessageList.svelte
│           ├── ChatInput.svelte
│           ├── ServerSidebar.svelte
│           ├── ChannelList.svelte
│           ├── UserAvatar.svelte
│           ├── PresenceIndicator.svelte
│           └── FriendSuggestions.svelte
```

---

## Phase 2: Collaborative Posts (CRDT)

**Status**: Not started
**Goal**: Learn CRDTs by implementing Google Docs-style co-editing for server posts.
**Depends on**: Phase 1 complete

### 2.1 CRDT Integration

- Integrate `yrs` crate into the backend
- Each collaborative post is a `Y.Doc` instance
- Sync protocol: Yjs sync v1 (state vectors + update deltas) over WebSocket
- Backend acts as the authoritative CRDT peer — merges all updates, persists state
- On client connect: send full state vector, then incremental updates

### 2.2 Post Lifecycle

1. **Create draft** → spawns a Y.Doc, opens a collab WS room
2. **Invite collaborators** → graph determines eligible users (friends or server members)
3. **Co-edit** → all connected users see real-time changes, cursors, selections
4. **Publish** → freeze Y.Doc, extract final content, store as immutable `post` record
5. **View** → published posts display in server feed, ranked by graph distance

### 2.3 Awareness Protocol

- Cursor positions broadcast to all collaborators
- Selection highlights in each collaborator's assigned color
- User list showing who's currently editing
- Idle/active status within the editing session

### 2.4 New Files

**Backend:**
- `src/collab/mod.rs` — CRDT document manager (HashMap of active Y.Docs)
- `src/collab/doc.rs` — Y.Doc lifecycle: create, apply update, encode state, persist
- `src/collab/awareness.rs` — cursor/selection state broadcasting
- `src/handlers/posts.rs` — post CRUD + publish endpoint

**Frontend:**
- `CollabEditor.svelte` — rich text editor (TipTap + yjs bindings)
- `CollaboratorCursors.svelte` — overlay showing other users' cursors
- `CollabInvite.svelte` — invite friends/members to co-edit
- `PostCard.svelte` — published post display in feed

### 2.5 Verification Checklist

- [ ] Create a draft post → Y.Doc initialized on server
- [ ] Two users open the same draft → edits sync in real-time
- [ ] Cursor positions visible for all collaborators
- [ ] Publish post → content frozen, collab room closed
- [ ] Published post appears in server feed
- [ ] Disconnect and reconnect → state catches up via sync protocol
- [ ] Only eligible users (friends/members) can be invited

---

## Phase 3: Shared Whiteboard

**Status**: Not started
**Goal**: Hardest state-sync problem — arbitrary canvas operations via CRDTs.
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

### 3.3 Layers and Z-Ordering

- `Y.Array` per layer
- Z-ordering via array position
- Layer visibility toggles (client-side only)
- Lock layers to prevent editing

### 3.4 Persistence

- Y.Doc state encoded and stored in SurrealDB on interval (every 30s) and on last-user-disconnect
- On first user connect: load from DB, hydrate Y.Doc
- History: store periodic snapshots for undo-to-checkpoint

### 3.5 New Files

**Backend:**
- `src/collab/whiteboard.rs` — whiteboard-specific CRDT logic, shape types

**Frontend:**
- `Whiteboard.svelte` — main canvas component (HTML5 Canvas)
- `DrawingTools.svelte` — toolbar for pen/shapes/eraser
- `WhiteboardLayer.svelte` — layer panel
- `WhiteboardCursors.svelte` — show other users' cursor positions on canvas

### 3.6 Verification Checklist

- [ ] Draw on whiteboard in tab A → appears in tab B in real-time
- [ ] Multiple simultaneous drawers produce correct merged result
- [ ] Reload page → whiteboard state loads from DB
- [ ] Eraser removes strokes for all users
- [ ] Select and move a shape → position updates for everyone
- [ ] Layer ordering works correctly

---

## Phase 4: Watch-Together Rooms

**Status**: Not started
**Goal**: Synchronized shared experience with graph-powered recommendations.
**Depends on**: Phase 1 complete (can run parallel to Phases 2-3)

### 4.1 Watch Room as Channel Type

- New channel type: `watch`
- Room state: current media URL, playback position, playing/paused, queue
- State synced via WebSocket (not CRDT — simpler leader-based sync)

### 4.2 Playback Sync

- **Leader model**: room owner or designated leader controls playback
- Leader actions (play, pause, seek) broadcast to all members
- Clients adjust playback to match leader's timestamp (with latency compensation)
- Periodic sync pulses every 5 seconds to correct drift

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
- [ ] Seek to timestamp → all members jump to correct position
- [ ] Add item to queue → appears for all members
- [ ] Vote on queue item → order updates in real-time
- [ ] Emoji reactions visible for all room members
- [ ] Recommendations improve as more watch history accumulates
- [ ] New member joining mid-playback syncs to correct position

---

## Milestone Summary

| Phase | Core Learning | Key Tech | Estimated Scope |
|-------|--------------|----------|-----------------|
| 1 | Graph modeling, WebSocket fundamentals | SurrealDB graphs, Axum WS, Tokio actors | Foundation — largest phase |
| 2 | CRDTs, collaborative editing | Yrs, TipTap, Yjs sync protocol | Medium — builds on Phase 1 WS infra |
| 3 | Complex state sync, canvas | Yrs + Canvas API | Medium — builds on Phase 2 CRDT infra |
| 4 | Graph algorithms, media sync | Graph traversals, leader-based sync | Medium — partially independent |
