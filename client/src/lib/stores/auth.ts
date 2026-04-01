import { writable } from 'svelte/store';

interface User {
  id: string;
  username: string;
  displayName: string;
}

interface AuthState {
  token: string | null;
  user: User | null;
  isAuthenticated: boolean;
}

const initialState: AuthState = {
  token: null,
  user: null,
  isAuthenticated: false,
};

export const auth = writable<AuthState>(initialState);
