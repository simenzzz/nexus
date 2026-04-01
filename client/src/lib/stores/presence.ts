import { writable } from 'svelte/store';

export type UserStatus = 'online' | 'idle' | 'dnd' | 'offline';

interface PresenceState {
  statuses: Map<string, UserStatus>;
}

export const presence = writable<PresenceState>({
  statuses: new Map(),
});
