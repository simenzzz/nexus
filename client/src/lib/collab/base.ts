import * as Y from 'yjs';
import { wsClient, type WsMessage } from '../ws/client';

/**
 * Per-resource-kind WS message prefix + id field. Phase 2 posts use the
 * "collab" prefix with `post_id`; Phase 3 whiteboards use "whiteboard" with
 * `whiteboard_id`. Adding a new resource kind is a single config object.
 *
 * The awareness type names are tracked separately because Phase 2 shipped
 * with the unprefixed `awareness_state` / `awareness_update` (post-only),
 * while Phase 3 uses fully prefixed `whiteboard_awareness_*`. Branching on
 * `prefix === 'collab'` in code would silently break the Phase 2 wire
 * format if the prefix were ever renamed — so we wire it explicitly.
 */
export interface ProviderConfig {
  /** Message type prefix — e.g. "collab" or "whiteboard". */
  prefix: string;
  /** JSON key carrying the resource id — e.g. "post_id" or "whiteboard_id". */
  idField: string;
  /** Concrete resource id. */
  id: string;
  /**
   * WS message type for inbound awareness state. Phase 2 = `awareness_state`,
   * Phase 3 = `whiteboard_awareness_state`.
   */
  awarenessStateType: string;
  /**
   * WS message type for outbound awareness updates. Phase 2 =
   * `awareness_update`, Phase 3 = `whiteboard_awareness_update`.
   */
  awarenessUpdateType: string;
}

/**
 * Common WS-bound Y.Doc plumbing shared by the collab + whiteboard providers.
 * Concrete subclasses bind their own typed roots (Y.Text, Y.Array<Y.Map>, …)
 * and expose resource-specific helpers.
 *
 * - Sends local Y.Doc updates over the WS bridge as `{prefix}_update`.
 * - Applies remote `{prefix}_state` / `{prefix}_update` payloads back into the
 *   doc (tagged with a remote origin so we don't echo them).
 * - Tracks awareness state from `{prefix}_awareness_state` and exposes a
 *   subscriber API.
 * - On `{prefix}_closed`, fires the closed handlers so the UI can re-subscribe
 *   or surface the reason (used by the checkpoint-restore flow).
 */
export abstract class BaseProvider {
  readonly doc: Y.Doc;
  protected readonly cfg: ProviderConfig;
  protected readonly cleanups: Array<() => void> = [];
  protected readonly remoteOrigin = Symbol('remote');
  private awarenessHandlers: Array<(users: Record<string, unknown>) => void> = [];
  private peerCount = 0;
  private peerCountHandlers: Array<(count: number) => void> = [];
  private closedHandlers: Array<(reason: string) => void> = [];

  protected constructor(cfg: ProviderConfig) {
    this.cfg = cfg;
    this.doc = new Y.Doc();

    const onLocalUpdate = (update: Uint8Array, origin: unknown) => {
      if (origin === this.remoteOrigin) return;
      wsClient.send({
        v: 1,
        type: `${cfg.prefix}_update`,
        [cfg.idField]: cfg.id,
        update_b64: bytesToBase64(update),
      });
    };
    this.doc.on('update', onLocalUpdate);
    this.cleanups.push(() => this.doc.off('update', onLocalUpdate));

    this.cleanups.push(
      wsClient.on(`${cfg.prefix}_state`, (msg: WsMessage) => {
        if (msg[cfg.idField] !== cfg.id) return;
        const state = base64ToBytes(msg.state_b64 as string);
        Y.applyUpdate(this.doc, state, this.remoteOrigin);
      }),
    );

    this.cleanups.push(
      wsClient.on(`${cfg.prefix}_update`, (msg: WsMessage) => {
        if (msg[cfg.idField] !== cfg.id) return;
        const update = base64ToBytes(msg.update_b64 as string);
        Y.applyUpdate(this.doc, update, this.remoteOrigin);
      }),
    );

    this.cleanups.push(
      wsClient.on(cfg.awarenessStateType, (msg: WsMessage) => {
        if (msg[cfg.idField] !== cfg.id) return;
        const users = (msg.users as Record<string, unknown>) ?? {};
        this.peerCount = Object.keys(users).length;
        for (const h of this.awarenessHandlers) h(users);
        for (const h of this.peerCountHandlers) h(this.peerCount);
      }),
    );

    this.cleanups.push(
      wsClient.on(`${cfg.prefix}_error`, (msg: WsMessage) => {
        if (msg[cfg.idField] !== cfg.id) return;
        console.error(`[${cfg.prefix}]`, msg.code, msg.message);
      }),
    );

    this.cleanups.push(
      wsClient.on(`${cfg.prefix}_closed`, (msg: WsMessage) => {
        if (msg[cfg.idField] !== cfg.id) return;
        const reason = (msg.reason as string) ?? '';
        for (const h of this.closedHandlers) h(reason);
      }),
    );

    wsClient.send({
      v: 1,
      type: `${cfg.prefix}_subscribe`,
      [cfg.idField]: cfg.id,
    });
  }

  /**
   * Broadcast an opaque awareness blob (cursor pos, tool, color, etc.).
   * The outbound message type is configured per resource kind via
   * [`ProviderConfig.awarenessUpdateType`].
   */
  sendAwareness(state: Record<string, unknown>): void {
    wsClient.send({
      v: 1,
      type: this.cfg.awarenessUpdateType,
      [this.cfg.idField]: this.cfg.id,
      state,
    });
  }

  onAwareness(handler: (users: Record<string, unknown>) => void): () => void {
    this.awarenessHandlers.push(handler);
    return () => {
      this.awarenessHandlers = this.awarenessHandlers.filter((h) => h !== handler);
    };
  }

  onPeerCount(handler: (count: number) => void): () => void {
    this.peerCountHandlers.push(handler);
    handler(this.peerCount);
    return () => {
      this.peerCountHandlers = this.peerCountHandlers.filter((h) => h !== handler);
    };
  }

  /**
   * Fires when the server tears down the session — for posts that means the
   * post was published; for whiteboards it means a checkpoint was restored
   * and the client should re-subscribe to fetch the new state.
   */
  onClosed(handler: (reason: string) => void): () => void {
    this.closedHandlers.push(handler);
    return () => {
      this.closedHandlers = this.closedHandlers.filter((h) => h !== handler);
    };
  }

  destroy(): void {
    wsClient.send({
      v: 1,
      type: `${this.cfg.prefix}_unsubscribe`,
      [this.cfg.idField]: this.cfg.id,
    });
    for (const cleanup of this.cleanups) cleanup();
    this.doc.destroy();
  }
}

export function bytesToBase64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.byteLength; i++) {
    s += String.fromCharCode(bytes[i]);
  }
  return btoa(s);
}

export function base64ToBytes(b64: string): Uint8Array {
  const s = atob(b64);
  const bytes = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
  return bytes;
}
