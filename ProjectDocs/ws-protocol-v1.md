# WebSocket Protocol v1 Specification

## Design Principles

1. Every event has a version field (`"v": 1`) for future protocol negotiation
2. Server-assigned monotonic sequence numbers per channel enable gap detection
3. Critical events get acknowledgments so clients can show pending/sent/failed states
4. Resume protocol avoids full re-fetch on reconnect
5. Subscription tiers reduce fan-out cost

---

## Message Envelope

All messages are JSON with these common fields:

```json
{
  "v": 1,
  "type": "...",
  "ts": 1712567890123
}
```

- `v` — protocol version (integer)
- `type` — message type discriminator (string)
- `ts` — server timestamp in epoch milliseconds (server-to-client only)

### Clock contract

All `ts` / `server_ts` fields are **server wall-clock** (`SystemTime::now()`
since UNIX epoch, in milliseconds — `chrono::Utc::now().timestamp_millis()`
on the server). Clients must not assume their local clock matches; in
particular the watch-room drift controller projects playback as
`position_ms + (now - server_ts) * rate` and falls back to the raw
`position_ms` when `now - server_ts` is negative or exceeds 10 s (sleep/
resume, queued message, or a clock skew large enough to make projection
meaningless). Servers should never overwrite `server_ts` on relay — it must
remain the originating server's emission timestamp.

---

## Client-to-Server Messages

### Authentication
```json
{"v": 1, "type": "auth", "ticket": "<ws-ticket>", "nonce": "<nonce>"}
```
First message after WS connect. Ticket and nonce are obtained via
`POST /api/auth/ws-ticket`. They are sent in the first WebSocket frame, not
the upgrade URL, so proxy access logs do not capture the one-time credential.
Server responds with `auth_ok` or closes the connection.

### Subscribe to Channel
```json
{"v": 1, "type": "subscribe", "channel_id": "channel:abc", "level": "active"}
```
Levels:
- `"active"` — full messages, typing indicators, presence for channel members
- `"badge"` — unread count updates only

### Unsubscribe
```json
{"v": 1, "type": "unsubscribe", "channel_id": "channel:abc"}
```

### Send Chat Message
```json
{"v": 1, "type": "chat_message", "channel_id": "channel:abc", "content": "Hello", "nonce": "local-uuid-123"}
```
`nonce` is a client-generated UUID for correlating the ACK back to the optimistic UI entry.

### Typing Indicator
```json
{"v": 1, "type": "typing", "channel_id": "channel:abc"}
```
Server debounces and forwards to active subscribers. Auto-expires after 3 seconds.

### Resume (on reconnect)
```json
{"v": 1, "type": "resume", "last_seq": {"channel:abc": 42, "channel:def": 108}}
```
Server replays missed events or sends `resync` if gap too large.

### Heartbeat
```json
{"v": 1, "type": "heartbeat"}
```
Client sends every 30 seconds. Server uses absence to detect idle (5min) or disconnect (60s).

---

## Server-to-Client Messages

### Auth OK
```json
{"v": 1, "type": "auth_ok", "user_id": "user:abc", "heartbeat_interval": 30000}
```

### Chat Message (broadcast)
```json
{
  "v": 1, "type": "chat_message", "seq": 43,
  "channel_id": "channel:abc", "message_id": "message:xyz",
  "author": {"id": "user:def", "username": "alice", "avatar_url": "..."},
  "content": "Hello", "ts": 1712567890123
}
```

### Message ACK (to sender only)
```json
{"v": 1, "type": "message_ack", "nonce": "local-uuid-123", "message_id": "message:xyz", "seq": 43, "ts": 1712567890123}
```
Client replaces pending message (matched by nonce) with confirmed message.

### Typing (broadcast to active subscribers)
```json
{"v": 1, "type": "typing", "channel_id": "channel:abc", "user_id": "user:def", "username": "alice"}
```

### Presence Update
```json
{"v": 1, "type": "presence", "user_id": "user:def", "status": "online"}
```
Only sent to users who share a server or friendship with the target user.

### Unread Badge (for badge-level subscribers)
```json
{"v": 1, "type": "unread", "channel_id": "channel:abc", "count": 5, "last_message_preview": "Hey everyone..."}
```

### Resync (gap too large to replay)
```json
{"v": 1, "type": "resync", "channel_id": "channel:abc"}
```
Client should fetch recent messages via REST and reset its local seq tracker.

### Heartbeat ACK
```json
{"v": 1, "type": "heartbeat_ack"}
```

---

## Sequence Numbers

- Each channel has an independent monotonic sequence counter
- Stored in Redis: `channel:{id}:seq` (INCR on each message)
- On reconnect, client sends `last_seq` per channel
- Server replays events from `last_seq + 1` to current
- If the gap exceeds a threshold (e.g., 500 events), server sends `resync` instead

---

## Connection Lifecycle

```
Client                          Server
  │                               │
  │──── WS Connect ──────────────▶│
  │                               │
  │──── auth {ticket, nonce} ────▶│  validate ticket (Redis, single-use)
  │◀─── auth_ok ─────────────────│
  │                               │
  │──── resume {last_seq} ───────▶│  replay or resync per channel
  │◀─── [missed events] ────────│
  │                               │
  │──── subscribe {active} ──────▶│  join room actor
  │◀─── [messages, typing, etc.] │
  │                               │
  │──── heartbeat ───────────────▶│  reset idle timer
  │◀─── heartbeat_ack ──────────│
  │                               │
  │ .... (normal operation) ....  │
  │                               │
  │──── [connection drops] ──────▶│  30s grace, then mark offline
  │                               │
  │──── WS Reconnect ───────────▶│  exponential backoff: 1s, 2s, 4s... max 30s
  │──── auth + resume ───────────▶│
```

---

## Rate Limits (WS)

- `chat_message`: 5 per 5 seconds per user per channel
- `typing`: 1 per 3 seconds per channel (server debounces)
- `subscribe`: 10 per second
- `heartbeat`: 1 per 20 seconds minimum interval

Exceeding limits results in a warning message; persistent abuse disconnects the client.
