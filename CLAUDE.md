# Nexus — Project Conventions

## Overview

Social media / Discord hybrid built with Rust (Axum) + SvelteKit + SurrealDB.
See `roadmaps/nexus.md` for the full phased roadmap (Phases 0-4).

## Tech Stack

- **Backend**: Rust, Axum, Tokio, SurrealDB Rust SDK, Yrs (CRDTs)
- **Frontend**: SvelteKit, TypeScript
- **Database**: SurrealDB (graph edges + document storage)
- **Cache/Ephemeral**: Redis (presence, rate limits, refresh tokens, WS tickets)
- **Auth**: JWT (jsonwebtoken crate) — dual-token (access 15min + refresh 7d)

## Project Structure

```
server/          # Rust backend (Cargo workspace)
client/          # SvelteKit frontend
roadmaps/        # Phased roadmap (nexus.md — Phases 0-4)
ProjectDocs/     # Architecture decisions, protocol specs, risk analysis
```

## Rust Conventions

### Error Handling
- Use `thiserror` for library-style errors (typed, matchable)
- Use `anyhow` sparingly — only in main.rs or one-off scripts
- Every handler returns `Result<Json<T>, AppError>` where `AppError` implements `IntoResponse`
- Never use `.unwrap()` in production paths — use `?` or explicit error handling

### Immutability
- Prefer owned types returned from functions over mutating references
- Use `Clone` + transform over in-place mutation
- Structs should derive `Clone` when reasonable

### Naming
- Modules: `snake_case`
- Types/Traits: `PascalCase`
- Functions: `snake_case`
- Constants: `SCREAMING_SNAKE_CASE`

### File Size
- Target 200-400 lines per file, hard max 800
- Split large modules into submodules

## SurrealDB Conventions

### Record IDs
- Use SurrealDB's built-in record IDs: `user:abc123`
- Never store IDs as plain strings — use the typed `Thing` from the SDK

### Graph Edges
- Create edges with `RELATE`: `RELATE user:a -> follows -> user:b`
- Edge tables are lowercase: `follows`, `friends_with`, `member_of`
- Store metadata on edges: `RELATE user:a -> member_of -> server:x SET role = 'admin', joined_at = time::now()`

### Queries
- Use parameterized queries — never interpolate user input into SurrealQL
- Prefer graph traversal syntax (`->edge->table`) over JOINs
- Index frequently traversed edges

## WebSocket Conventions

### Protocol v1
All WS messages include `"v": 1` and a `type` discriminator. Server messages include `seq` (per-channel sequence number) and `ts`. See `ProjectDocs/ws-protocol-v1.md` for full spec.

### Room Actor Pattern
- Each channel spawns a Tokio task acting as a room actor
- Communication via `tokio::sync::mpsc` channels
- Room holds a map of `UserId -> mpsc::Sender` for connected clients
- Room shuts down when last client disconnects (with grace period)

### Reconnection
- Client implements exponential backoff: 1s, 2s, 4s, 8s, max 30s
- On reconnect, client sends `resume` with last known seq per channel
- Server replays missed events or sends `resync` if gap too large

## Frontend Conventions

### Stores
- Use Svelte stores for shared state (auth, chat, presence)
- Stores are the single source of truth — components read from stores, WS client writes to stores

### API Client
- All REST calls go through `lib/api/client.ts`
- Handles JWT refresh, error normalization
- Returns typed responses

### Components
- One component per file
- Props are typed via TypeScript
- Events use Svelte's `createEventDispatcher` or callback props

## Running the Project

```bash
# Database
surreal start --user root --pass root memory  # or file://nexus.db for persistence

# Backend
cd server && cargo run

# Frontend
cd client && npm install && npm run dev
```

## Testing

- **Rust**: `cargo test` — unit tests in each module, integration tests in `tests/`
- **Frontend**: Vitest for unit tests, Playwright for E2E
- **Target**: 80% coverage minimum
- **TDD workflow**: write test (RED) → implement (GREEN) → refactor (IMPROVE)

## Architectural Decisions

Key decisions documented in `ProjectDocs/architectural-decisions.md`:
- **ADR-001**: Full repository trait pattern (handlers use `Arc<dyn XRepo>`, not raw DB)
- **ADR-002**: Redis from day one (presence, rate limits, refresh tokens, WS tickets)
- **ADR-003**: WS protocol + Auth + Rate limiting built before business logic

## Security Checklist

Before any commit:
- [ ] No hardcoded secrets — use environment variables
- [ ] All SurrealQL queries are parameterized
- [ ] User input validated at handler boundaries
- [ ] JWT tokens have reasonable expiry
- [ ] WebSocket connections authenticated via single-use tickets (not raw JWT)
- [ ] Rate limiting on all public endpoints
