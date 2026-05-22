-- Nexus schema bootstrap.
-- Idempotent: every DEFINE uses IF NOT EXISTS so re-runs on container start
-- are safe. Tables we want to validate against (auth + edges) are SCHEMAFULL;
-- payload-heavy tables (Yjs blobs, queue items) are SCHEMALESS so a future
-- field addition doesn't need a migration.
--
-- Run via:
--   surreal import --endpoint http://surrealdb:8000 --user $SURREAL_USER \
--     --pass $SURREAL_PASS --namespace nexus --database nexus /db/init.surql
--
-- This script assumes the namespace and database have been pre-selected
-- (the import command does that). Application code further validates input
-- at the handler boundary — these constraints are defense-in-depth.

-- ─── Users ─────────────────────────────────────────────────────────────────
DEFINE TABLE IF NOT EXISTS user SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS username ON user TYPE string
    ASSERT $value != NONE AND string::len($value) >= 3 AND string::len($value) <= 32;
DEFINE FIELD IF NOT EXISTS display_name ON user TYPE string
    ASSERT $value != NONE AND string::len($value) <= 64;
DEFINE FIELD IF NOT EXISTS avatar_url ON user TYPE option<string>;
DEFINE FIELD IF NOT EXISTS password_hash ON user TYPE string ASSERT $value != NONE;
DEFINE FIELD IF NOT EXISTS status ON user TYPE string
    ASSERT $value INSIDE ['online', 'idle', 'dnd', 'offline'];
DEFINE FIELD IF NOT EXISTS created_at ON user TYPE datetime;
DEFINE INDEX IF NOT EXISTS user_username_unique ON user FIELDS username UNIQUE;

-- ─── Servers ───────────────────────────────────────────────────────────────
DEFINE TABLE IF NOT EXISTS server SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON server TYPE string
    ASSERT $value != NONE AND string::len($value) >= 1 AND string::len($value) <= 64;
DEFINE FIELD IF NOT EXISTS description ON server TYPE option<string>
    ASSERT $value = NONE OR string::len($value) <= 1024;
DEFINE FIELD IF NOT EXISTS icon_url ON server TYPE option<string>;
DEFINE FIELD IF NOT EXISTS owner ON server TYPE record<user>;
DEFINE FIELD IF NOT EXISTS created_at ON server TYPE datetime;

-- ─── Channels ──────────────────────────────────────────────────────────────
DEFINE TABLE IF NOT EXISTS channel SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name ON channel TYPE string
    ASSERT $value != NONE AND string::len($value) >= 1 AND string::len($value) <= 64;
DEFINE FIELD IF NOT EXISTS channel_type ON channel TYPE string
    ASSERT $value INSIDE ['text', 'voice', 'collab', 'whiteboard', 'watch'];
DEFINE FIELD IF NOT EXISTS server ON channel TYPE record<server>;
DEFINE FIELD IF NOT EXISTS created_at ON channel TYPE datetime;
DEFINE INDEX IF NOT EXISTS channel_server_idx ON channel FIELDS server;

-- ─── Messages (chat) ───────────────────────────────────────────────────────
-- SCHEMALESS so future reaction/attachment fields don't require a migration.
DEFINE TABLE IF NOT EXISTS message SCHEMALESS;
DEFINE FIELD IF NOT EXISTS content ON message TYPE string
    ASSERT $value != NONE AND string::len($value) >= 1 AND string::len($value) <= 4000;
DEFINE FIELD IF NOT EXISTS author ON message TYPE record<user>;
DEFINE FIELD IF NOT EXISTS channel ON message TYPE record<channel>;
DEFINE FIELD IF NOT EXISTS created_at ON message TYPE datetime;
DEFINE INDEX IF NOT EXISTS message_channel_created_idx ON message FIELDS channel, created_at;

-- ─── Posts (collaborative drafts) ──────────────────────────────────────────
DEFINE TABLE IF NOT EXISTS post SCHEMALESS;
DEFINE FIELD IF NOT EXISTS author ON post TYPE record<user>;
DEFINE FIELD IF NOT EXISTS title ON post TYPE string
    ASSERT $value != NONE AND string::len($value) <= 200;
DEFINE FIELD IF NOT EXISTS published ON post TYPE bool;
DEFINE INDEX IF NOT EXISTS post_author_idx ON post FIELDS author;
DEFINE INDEX IF NOT EXISTS post_published_idx ON post FIELDS published;

-- ─── Whiteboards (shared canvas) ───────────────────────────────────────────
-- 1:1 with channel; the application keys by `channel` so we index it.
DEFINE TABLE IF NOT EXISTS whiteboard SCHEMALESS;
DEFINE FIELD IF NOT EXISTS channel ON whiteboard TYPE record<channel>;
DEFINE INDEX IF NOT EXISTS whiteboard_channel_unique ON whiteboard FIELDS channel UNIQUE;

DEFINE TABLE IF NOT EXISTS whiteboard_checkpoint SCHEMALESS;
DEFINE FIELD IF NOT EXISTS channel ON whiteboard_checkpoint TYPE record<channel>;
DEFINE INDEX IF NOT EXISTS whiteboard_checkpoint_channel_idx
    ON whiteboard_checkpoint FIELDS channel, created_at;

-- ─── Watch rooms (synchronized media) ──────────────────────────────────────
DEFINE TABLE IF NOT EXISTS watch_room SCHEMALESS;
DEFINE FIELD IF NOT EXISTS channel ON watch_room TYPE record<channel>;
DEFINE INDEX IF NOT EXISTS watch_room_channel_unique ON watch_room FIELDS channel UNIQUE;

DEFINE TABLE IF NOT EXISTS queue_item SCHEMALESS;
DEFINE FIELD IF NOT EXISTS room ON queue_item TYPE record<watch_room>;
DEFINE FIELD IF NOT EXISTS video_id ON queue_item TYPE string
    ASSERT $value != NONE AND string::len($value) = 11;
-- string::len counts bytes; the WS boundary in connection.rs caps on chars
-- to keep the user-visible limit predictable. We give the DB headroom (4 ×
-- 200 = 800) so a 200-char emoji-heavy title passes — boundary check is the
-- canonical limit.
DEFINE FIELD IF NOT EXISTS title ON queue_item TYPE string
    ASSERT string::len($value) <= 800;
DEFINE FIELD IF NOT EXISTS added_by ON queue_item TYPE record<user>;
DEFINE FIELD IF NOT EXISTS score ON queue_item TYPE number DEFAULT 0;
DEFINE INDEX IF NOT EXISTS queue_item_room_idx ON queue_item FIELDS room, score, created_at;

-- ─── Social graph (edges) ──────────────────────────────────────────────────
-- All edges are SCHEMALESS so per-edge metadata (status, role, etc.) can be
-- added without a migration.
DEFINE TABLE IF NOT EXISTS friends_with SCHEMALESS;
DEFINE INDEX IF NOT EXISTS friends_with_in_idx ON friends_with FIELDS in;
DEFINE INDEX IF NOT EXISTS friends_with_out_idx ON friends_with FIELDS out;

DEFINE TABLE IF NOT EXISTS follows SCHEMALESS;
DEFINE INDEX IF NOT EXISTS follows_in_idx ON follows FIELDS in;
DEFINE INDEX IF NOT EXISTS follows_out_idx ON follows FIELDS out;

DEFINE TABLE IF NOT EXISTS blocked SCHEMALESS;
DEFINE INDEX IF NOT EXISTS blocked_in_idx ON blocked FIELDS in;
DEFINE INDEX IF NOT EXISTS blocked_out_idx ON blocked FIELDS out;

DEFINE TABLE IF NOT EXISTS member_of SCHEMALESS;
DEFINE INDEX IF NOT EXISTS member_of_in_idx ON member_of FIELDS in;
DEFINE INDEX IF NOT EXISTS member_of_out_idx ON member_of FIELDS out;

DEFINE TABLE IF NOT EXISTS invited_to SCHEMALESS;
DEFINE INDEX IF NOT EXISTS invited_to_in_idx ON invited_to FIELDS in;
DEFINE INDEX IF NOT EXISTS invited_to_out_idx ON invited_to FIELDS out;

-- ─── Watch-graph edges ─────────────────────────────────────────────────────
DEFINE TABLE IF NOT EXISTS votes SCHEMALESS;
DEFINE INDEX IF NOT EXISTS votes_in_idx ON votes FIELDS in;
DEFINE INDEX IF NOT EXISTS votes_out_idx ON votes FIELDS out;

DEFINE TABLE IF NOT EXISTS watched SCHEMALESS;
DEFINE INDEX IF NOT EXISTS watched_in_idx ON watched FIELDS in;
DEFINE INDEX IF NOT EXISTS watched_out_idx ON watched FIELDS out;

-- ─── Done ──────────────────────────────────────────────────────────────────
INFO FOR DB;
