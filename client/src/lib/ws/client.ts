import { browser } from '$app/environment';

const WS_URL = import.meta.env.PUBLIC_WS_URL ?? 'ws://localhost:3001/ws';

export interface WsMessage {
  type: string;
  [key: string]: unknown;
}

type MessageHandler = (message: WsMessage) => void;

class WebSocketClient {
  private ws: WebSocket | null = null;
  private handlers: Map<string, MessageHandler[]> = new Map();
  private reconnectAttempts = 0;
  private maxReconnectDelay = 30_000;
  private token: string | null = null;

  connect(token: string): void {
    if (!browser) return;

    this.token = token;
    this.ws = new WebSocket(`${WS_URL}?token=${token}`);

    this.ws.onopen = () => {
      this.reconnectAttempts = 0;
      console.log('WebSocket connected');
    };

    this.ws.onmessage = (event: MessageEvent) => {
      const message: WsMessage = JSON.parse(event.data as string);
      const handlers = this.handlers.get(message.type) ?? [];
      for (const handler of handlers) {
        handler(message);
      }
    };

    this.ws.onclose = () => {
      console.log('WebSocket disconnected');
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      this.ws?.close();
    };
  }

  private scheduleReconnect(): void {
    if (!this.token) return;

    const delay = Math.min(
      1000 * Math.pow(2, this.reconnectAttempts),
      this.maxReconnectDelay,
    );
    this.reconnectAttempts++;
    console.log(`Reconnecting in ${delay}ms (attempt ${this.reconnectAttempts})`);
    setTimeout(() => {
      if (this.token) this.connect(this.token);
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
    this.token = null;
    this.ws?.close();
    this.ws = null;
  }
}

export const wsClient = new WebSocketClient();
