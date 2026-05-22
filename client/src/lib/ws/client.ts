import { browser, dev } from '$app/environment';
import { env } from '$env/dynamic/public';
import { api } from '$lib/api/client';
import { getLastSeqPerChannel } from '$stores/chat';

export interface WsMessage {
  v: number;
  type: string;
  [key: string]: unknown;
}

type MessageHandler = (message: WsMessage) => void;

function websocketUrl(): string {
  const configured = env.PUBLIC_WS_URL?.trim();
  if (configured) return configured;
  if (dev) return 'ws://localhost:3001/ws';
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/ws`;
}

class WebSocketClient {
  private ws: WebSocket | null = null;
  private handlers: Map<string, MessageHandler[]> = new Map();
  private reconnectAttempts = 0;
  private maxReconnectDelay = 30_000;
  private heartbeatInterval: ReturnType<typeof setInterval> | null = null;
  private shouldReconnect = false;
  private hasConnectedBefore = false;

  async connect(): Promise<void> {
    if (!browser) return;
    // Guard against stacking concurrent connections
    if (
      this.ws &&
      (this.ws.readyState === WebSocket.CONNECTING || this.ws.readyState === WebSocket.OPEN)
    ) {
      return;
    }
    this.shouldReconnect = true;

    try {
      const { ticket, nonce } = await api.post<{ ticket: string; nonce: string }>(
        '/api/auth/ws-ticket'
      );
      this.doConnect(ticket, nonce);
    } catch (err) {
      console.error('Failed to get WS ticket:', err);
      this.scheduleReconnect();
    }
  }

  private doConnect(ticket: string, nonce: string): void {
    this.ws = new WebSocket(websocketUrl());

    this.ws.onopen = () => {
      this.reconnectAttempts = 0;
      console.log('WebSocket connected');
      this.send({ v: 1, type: 'auth', ticket, nonce });

      // On reconnect, send resume with last known seq per channel
      if (this.hasConnectedBefore) {
        const lastSeq = getLastSeqPerChannel();
        if (Object.keys(lastSeq).length > 0) {
          this.send({
            v: 1,
            type: 'resume',
            last_seq: lastSeq,
          });
        }
      }
      this.hasConnectedBefore = true;
    };

    this.ws.onmessage = (event: MessageEvent) => {
      let message: WsMessage;
      try {
        message = JSON.parse(event.data as string);
      } catch {
        console.error('Failed to parse WS message:', event.data);
        return;
      }

      if (message.type === 'auth_ok') {
        this.startHeartbeat(message.heartbeat_interval as number);
        return;
      }

      if (message.type === 'heartbeat_ack') {
        return;
      }

      const handlers = this.handlers.get(message.type) ?? [];
      for (const handler of handlers) {
        handler(message);
      }
      // Also call wildcard handlers
      const wildcardHandlers = this.handlers.get('*') ?? [];
      for (const handler of wildcardHandlers) {
        handler(message);
      }
    };

    this.ws.onclose = () => {
      console.log('WebSocket disconnected');
      this.stopHeartbeat();
      if (this.shouldReconnect) {
        this.scheduleReconnect();
      }
    };

    this.ws.onerror = () => {
      this.ws?.close();
    };
  }

  private startHeartbeat(intervalMs: number): void {
    this.stopHeartbeat();
    const interval = intervalMs && intervalMs > 0 ? intervalMs : 30000;
    this.heartbeatInterval = setInterval(() => {
      this.send({ v: 1, type: 'heartbeat' });
    }, interval);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
    }
  }

  private scheduleReconnect(): void {
    if (!this.shouldReconnect) return;
    const delay = Math.min(
      1000 * Math.pow(2, this.reconnectAttempts),
      this.maxReconnectDelay,
    );
    this.reconnectAttempts++;
    console.log(`Reconnecting in ${delay}ms (attempt ${this.reconnectAttempts})`);
    setTimeout(() => {
      this.connect();
    }, delay);
  }

  on(type: string, handler: MessageHandler): () => void {
    const existing = this.handlers.get(type) ?? [];
    this.handlers.set(type, [...existing, handler]);

    return () => {
      const current = this.handlers.get(type) ?? [];
      this.handlers.set(
        type,
        current.filter((h) => h !== handler),
      );
    };
  }

  send(message: WsMessage): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(message));
    }
  }

  disconnect(): void {
    this.shouldReconnect = false;
    this.stopHeartbeat();
    this.ws?.close();
    this.ws = null;
  }
}

export const wsClient = new WebSocketClient();
