import { writable } from 'svelte/store';

interface Message {
  id: string;
  content: string;
  authorId: string;
  channelId: string;
  createdAt: string;
}

interface ChatState {
  messages: Message[];
  activeChannelId: string | null;
}

export const chat = writable<ChatState>({
  messages: [],
  activeChannelId: null,
});
