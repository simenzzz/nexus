import { writable } from 'svelte/store';
import { api } from '$lib/api/client';

export interface Channel {
  id: { id?: string; tb?: string } | null;
  name: string;
  channel_type: 'text' | 'voice' | 'collab' | 'whiteboard' | 'watch';
  server: { id?: string; tb?: string } | null;
  created_at?: string | null;
}

function extractId(recordId: { id?: string; tb?: string } | null): string {
  if (!recordId) return '';
  if (typeof recordId === 'string') return recordId;
  if (recordId.id && typeof recordId.id === 'string') return recordId.id;
  const inner = (recordId as Record<string, unknown>).id;
  if (inner && typeof inner === 'string') return inner;
  if (inner && typeof inner === 'object') {
    return String(Object.values(inner as Record<string, unknown>)[0] ?? '');
  }
  return String(recordId);
}

export const channels = writable<Map<string, Channel[]>>(new Map());

export async function fetchChannels(serverId: string): Promise<void> {
  try {
    const data = await api.get<{ channels: Channel[] }>(
      `/api/servers/${serverId}/channels`,
    );
    const list = data.channels;
    channels.update((map) => new Map(map).set(serverId, list));
  } catch (err) {
    console.error('Failed to fetch channels:', err);
  }
}
