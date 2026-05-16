import { writable } from 'svelte/store';
import { api } from '$lib/api/client';

/** Snapshot + metadata returned by `GET /api/channels/:id/whiteboard`. */
export interface WhiteboardSnapshot {
  channel_id: string;
  state_b64: string;
  state_vector_b64: string;
  snapshot_count: number;
}

export interface WhiteboardCheckpoint {
  id: { tb: string; id: { String?: string } } | string;
  channel: { tb: string; id: { String?: string } } | string;
  state_b64: string;
  label: string | null;
  created_at: string | null;
}

/** Current drawing tool. Persisted on the client, never shipped to the server. */
export type ToolKind =
  | 'pen'
  | 'rect'
  | 'circle'
  | 'line'
  | 'arrow'
  | 'text'
  | 'eraser'
  | 'select';

export interface ToolState {
  tool: ToolKind;
  color: string;
  strokeWidth: number;
  activeLayerId: string;
}

const DEFAULT_TOOL: ToolState = {
  tool: 'pen',
  color: '#222222',
  strokeWidth: 3,
  activeLayerId: 'default',
};

export const toolState = writable<ToolState>(DEFAULT_TOOL);

/** Visibility per layer is client-only (not in the CRDT). Keyed by layer id. */
export const layerVisibility = writable<Record<string, boolean>>({});

/** Surreal's REST encoding for record IDs (matches `posts.ts`). */
export function checkpointIdToString(cp: WhiteboardCheckpoint): string {
  const id = cp.id;
  if (typeof id === 'string') return id.replace(/^whiteboard_checkpoint:/, '');
  const inner = id?.id?.String;
  return inner ?? '';
}

export async function fetchWhiteboard(channelId: string): Promise<WhiteboardSnapshot> {
  return api.get<WhiteboardSnapshot>(
    `/api/channels/${encodeURIComponent(channelId)}/whiteboard`,
  );
}

export async function createCheckpoint(
  channelId: string,
  label?: string,
): Promise<WhiteboardCheckpoint> {
  const res = await api.post<{ checkpoint: WhiteboardCheckpoint }>(
    `/api/channels/${encodeURIComponent(channelId)}/whiteboard/checkpoints`,
    { label: label ?? null },
  );
  return res.checkpoint;
}

export async function listCheckpoints(channelId: string): Promise<WhiteboardCheckpoint[]> {
  const res = await api.get<{ checkpoints: WhiteboardCheckpoint[] }>(
    `/api/channels/${encodeURIComponent(channelId)}/whiteboard/checkpoints`,
  );
  return res.checkpoints;
}

export async function restoreCheckpoint(
  channelId: string,
  checkpointId: string,
): Promise<void> {
  await api.post(
    `/api/channels/${encodeURIComponent(channelId)}/whiteboard/checkpoints/${encodeURIComponent(checkpointId)}/restore`,
    {},
  );
}
