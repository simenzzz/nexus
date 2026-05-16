import { writable, derived } from 'svelte/store';
import { wsClient, type WsMessage } from '$lib/ws/client';

export interface WatchViewer {
  user_id: string;
  username: string;
  is_leader: boolean;
}

export interface WatchPlayback {
  video_id: string | null;
  position_ms: number;
  paused: boolean;
  server_ts: number;
  rate: number;
}

export interface WatchQueueItem {
  id: string;
  video_id: string;
  title: string;
  duration_ms: number;
  thumbnail_url: string | null;
  added_by: string;
  score: number;
}

export interface WatchRoomState {
  channel_id: string;
  leader_id: string | null;
  playback: WatchPlayback;
  queue: WatchQueueItem[];
  viewers: WatchViewer[];
  /// Most recent error message from the server (e.g. not_leader, rate_limited).
  error: string | null;
}

const initialPlayback: WatchPlayback = {
  video_id: null,
  position_ms: 0,
  paused: true,
  server_ts: Date.now(),
  rate: 1,
};

export const watchRooms = writable<Record<string, WatchRoomState>>({});

function update(channelId: string, patch: (s: WatchRoomState) => WatchRoomState): void {
  watchRooms.update((rooms) => {
    const existing = rooms[channelId] ?? {
      channel_id: channelId,
      leader_id: null,
      playback: { ...initialPlayback },
      queue: [],
      viewers: [],
      error: null,
    };
    return { ...rooms, [channelId]: patch(existing) };
  });
}

export function watchRoomStore(channelId: string) {
  return derived(watchRooms, ($r) => $r[channelId] ?? null);
}

/// Wire the WS handlers for a single watch channel. Returns an unsubscribe
/// that detaches every handler — call it on component teardown so we don't
/// leak across navigations.
export function bindWatchRoom(channelId: string): () => void {
  const offs: Array<() => void> = [];

  offs.push(
    wsClient.on('watch_state', (msg: WsMessage) => {
      if (msg.channel_id !== channelId) return;
      update(channelId, (s) => ({
        ...s,
        leader_id: (msg.leader_id as string | null) ?? null,
        playback: (msg.playback as WatchPlayback) ?? s.playback,
        queue: (msg.queue as WatchQueueItem[]) ?? [],
        viewers: (msg.viewers as WatchViewer[]) ?? [],
      }));
    }),
  );

  offs.push(
    wsClient.on('watch_playback', (msg: WsMessage) => {
      if (msg.channel_id !== channelId) return;
      const action = msg.action as 'play' | 'pause' | 'seek';
      update(channelId, (s) => ({
        ...s,
        playback: {
          ...s.playback,
          position_ms: msg.position_ms as number,
          server_ts: msg.server_ts as number,
          paused: action === 'pause' ? true : action === 'play' ? false : s.playback.paused,
        },
      }));
    }),
  );

  offs.push(
    wsClient.on('watch_sync_pulse', (msg: WsMessage) => {
      if (msg.channel_id !== channelId) return;
      update(channelId, (s) => ({
        ...s,
        playback: {
          ...s.playback,
          position_ms: msg.position_ms as number,
          server_ts: msg.server_ts as number,
          paused: msg.paused as boolean,
        },
      }));
    }),
  );

  offs.push(
    wsClient.on('watch_leader_changed', (msg: WsMessage) => {
      if (msg.channel_id !== channelId) return;
      const newLeader = msg.leader_id as string;
      update(channelId, (s) => ({
        ...s,
        leader_id: newLeader,
        viewers: s.viewers.map((v) => ({ ...v, is_leader: v.user_id === newLeader })),
      }));
    }),
  );

  offs.push(
    wsClient.on('watch_error', (msg: WsMessage) => {
      if (msg.channel_id !== channelId) return;
      update(channelId, (s) => ({
        ...s,
        error: `${msg.code as string}: ${msg.message as string}`,
      }));
    }),
  );

  wsClient.send({ v: 1, type: 'watch_subscribe', channel_id: channelId });

  return () => {
    wsClient.send({ v: 1, type: 'watch_unsubscribe', channel_id: channelId });
    for (const off of offs) off();
    watchRooms.update((rooms) => {
      const next = { ...rooms };
      delete next[channelId];
      return next;
    });
  };
}

/// Leader-only: send a playback control. `action` is "play" | "pause" | "seek".
export function sendPlayback(
  channelId: string,
  action: 'play' | 'pause' | 'seek',
  positionMs: number,
): void {
  wsClient.send({
    v: 1,
    type: 'watch_playback',
    channel_id: channelId,
    action,
    position_ms: Math.max(0, Math.round(positionMs)),
    client_ts: Date.now(),
  });
}

export function sendTransferLeader(channelId: string, toUserId: string): void {
  wsClient.send({
    v: 1,
    type: 'watch_transfer_leader',
    channel_id: channelId,
    to_user_id: toUserId,
  });
}
