import { writable } from 'svelte/store';
import { api } from '$lib/api/client';

export interface Post {
  id: { tb: string; id: { String?: string } } | string;
  author: { tb: string; id: { String?: string } } | string;
  title: string;
  state_b64: string;
  state_vector_b64: string;
  published: boolean;
  published_content: string | null;
  created_at: string | null;
  updated_at: string | null;
}

interface PostsState {
  byId: Record<string, Post>;
}

const initial: PostsState = { byId: {} };

export const posts = writable<PostsState>(initial);

/** Surreal's REST encoding for record IDs is `{ tb, id: { String: "..." } }`. */
export function postIdToString(post: Post): string {
  const id = post.id;
  if (typeof id === 'string') return id.replace(/^post:/, '');
  const inner = id?.id?.String;
  return inner ?? '';
}

export async function createDraft(title: string): Promise<Post> {
  const res = await api.post<{ post: Post }>('/api/posts', { title });
  posts.update((state) => ({
    byId: { ...state.byId, [postIdToString(res.post)]: res.post },
  }));
  return res.post;
}

export async function fetchPost(id: string): Promise<Post> {
  const res = await api.get<{ post: Post }>(`/api/posts/${id}`);
  posts.update((state) => ({
    byId: { ...state.byId, [postIdToString(res.post)]: res.post },
  }));
  return res.post;
}

export async function publishPost(id: string): Promise<Post> {
  const res = await api.post<{ post: Post }>(`/api/posts/${id}/publish`, {});
  posts.update((state) => ({
    byId: { ...state.byId, [postIdToString(res.post)]: res.post },
  }));
  return res.post;
}

export async function inviteCollaborator(postId: string, userId: string): Promise<void> {
  await api.post<{ ok: boolean }>(`/api/posts/${postId}/invites`, { user_id: userId });
}

export async function fetchPublishedPosts(): Promise<Post[]> {
  const res = await api.get<{ posts: Post[] }>('/api/posts');
  posts.update((state) => {
    const next = { ...state.byId };
    for (const p of res.posts) next[postIdToString(p)] = p;
    return { byId: next };
  });
  return res.posts;
}
