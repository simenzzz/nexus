# Nexus — Architecture Overview

## System Diagram

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

## Backend Architecture

The Rust backend uses **Axum** as the web framework, organized into layers:

- **Handlers** (`handlers/`): HTTP request handlers. Each returns `Result<Json<T>, AppError>`. Thin layer — validates input, calls services, returns responses.
- **Graph** (`graph/`): SurrealDB graph traversal queries for social features (friends, discovery, feed ranking).
- **WebSocket** (`ws/`): Real-time messaging using the room actor pattern. Each channel spawns a Tokio task that manages connected clients via `mpsc` channels.
- **Auth** (`auth/`): JWT token creation/validation and an Axum extractor middleware for authenticated routes.
- **Models** (`models/`): Data structures matching SurrealDB records. Uses `RecordId` for typed record references.
- **Config** (`config.rs`): Environment-based configuration loaded at startup.
- **Error** (`error.rs`): Unified `AppError` type implementing `IntoResponse` for consistent error responses.

## Database Model

SurrealDB serves as both document store and graph database.

**Records:**
- `user` — username, display_name, avatar_url, status
- `server` — name, description, icon_url, owner
- `channel` — name, channel_type, server reference
- `message` — content, author, channel, timestamps

**Graph Edges:**
- `follows` (user → user) — one-directional social follow
- `friends_with` (user ↔ user) — mutual friendship (two directed edges)
- `member_of` (user → server) — with role and joined_at metadata
- `blocked` (user → user) — block relationship

## Auth Flow

1. Client sends `POST /api/auth/login` with credentials
2. Server validates credentials against SurrealDB
3. Server returns JWT token (HS256, configurable expiry)
4. Client stores token and sends it as `Authorization: Bearer <token>` on all subsequent requests
5. `AuthUser` extractor middleware validates the token on protected routes
6. WebSocket connections authenticate with a short-lived ticket sent in the first WebSocket message

## WebSocket Architecture

Uses the **room actor pattern**:

1. Client connects to `GET /ws` (upgraded to WebSocket)
2. Client sends `Join { channel_id }` message
3. Server creates or finds the room actor (Tokio task) for that channel
4. Room holds a `HashMap<UserId, mpsc::Sender>` of connected clients
5. Messages broadcast to all clients in the room
6. On disconnect, client is removed from the room
7. Room shuts down when the last client disconnects (with grace period)

**Message types:** `ChatMessage`, `Typing`, `Presence`, `Join`, `Leave`

## Frontend Architecture

Built with **SvelteKit** and **Tailwind CSS v4**.

- **Routes**: Organized into `(auth)` and `(app)` groups. Auth pages are public; app pages require authentication.
- **Stores**: Svelte stores are the single source of truth for auth state, chat messages, and presence data.
- **API Client** (`lib/api/client.ts`): Centralized REST client handling JWT tokens and error normalization.
- **WS Client** (`lib/ws/client.ts`): WebSocket manager with automatic reconnection (exponential backoff: 1s → 30s max).
- **Components**: One component per file, typed props via TypeScript, Svelte 5 runes syntax.
