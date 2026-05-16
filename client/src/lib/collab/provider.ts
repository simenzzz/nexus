import * as Y from 'yjs';
import { BaseProvider } from './base';

const TEXT_ROOT = 'content';

/**
 * Binds a Y.Doc to the project's WS bridge for one post. Sends local updates
 * to the server, applies remote updates from peers, and exposes awareness
 * state via a callback.
 *
 * Concrete Phase 2 binding over the generic [`BaseProvider`]: pins the doc to
 * a single `Y.Text("content")` root and provides a one-shot `replaceText`
 * helper used by the editor's controlled-input bridge.
 */
export class CollabProvider extends BaseProvider {
  readonly text: Y.Text;

  constructor(postId: string) {
    super({
      prefix: 'collab',
      idField: 'post_id',
      id: postId,
      // Phase 2 wire format: unprefixed `awareness_*` types.
      awarenessStateType: 'awareness_state',
      awarenessUpdateType: 'awareness_update',
    });
    this.text = this.doc.getText(TEXT_ROOT);
  }

  /** Replace the entire text contents in a single transaction. */
  replaceText(next: string): void {
    this.doc.transact(() => {
      this.text.delete(0, this.text.length);
      this.text.insert(0, next);
    });
  }
}
