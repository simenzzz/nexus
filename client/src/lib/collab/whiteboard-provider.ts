import * as Y from 'yjs';
import { BaseProvider } from './base';

/** Stable root names. Server is a passive relay — only the client knows the schema. */
const LAYERS_ROOT = 'layers';
const SHAPES_BY_LAYER_ROOT = 'shapes_by_layer';
const META_ROOT = 'meta';

/** Built-in default layer id, created lazily on the first edit. */
export const DEFAULT_LAYER_ID = 'default';

export type ShapeType = 'pen' | 'rect' | 'circle' | 'line' | 'arrow' | 'text';

/** Single layer record stored inside `layers: Y.Array<Y.Map>`. */
export interface LayerMeta {
  id: string;
  name: string;
  locked: boolean;
  z_index: number;
}

/**
 * Y.Doc-bound whiteboard binding. The CRDT shape is opaque to the server —
 * shapes/layers/meta live entirely in Y roots defined here. Mutations should
 * be wrapped in [`transact`] so they ship as a single update to peers.
 */
export class WhiteboardProvider extends BaseProvider {
  readonly layers: Y.Array<Y.Map<unknown>>;
  readonly shapesByLayer: Y.Map<Y.Array<Y.Map<unknown>>>;
  readonly meta: Y.Map<unknown>;

  constructor(channelId: string) {
    super({
      prefix: 'whiteboard',
      idField: 'whiteboard_id',
      id: channelId,
      awarenessStateType: 'whiteboard_awareness_state',
      awarenessUpdateType: 'whiteboard_awareness_update',
    });
    this.layers = this.doc.getArray<Y.Map<unknown>>(LAYERS_ROOT);
    this.shapesByLayer = this.doc.getMap<Y.Array<Y.Map<unknown>>>(SHAPES_BY_LAYER_ROOT);
    this.meta = this.doc.getMap(META_ROOT);
  }

  /** Run a mutation inside a single Yjs transaction (one WS update). */
  transact(fn: () => void): void {
    this.doc.transact(fn);
  }

  /**
   * Return the shape array for a layer, creating both the layer entry and
   * the shape array if they don't exist yet. Called on first stroke per
   * session so an empty whiteboard doesn't require a manual "create layer".
   */
  ensureLayer(layerId: string, name = 'Layer'): Y.Array<Y.Map<unknown>> {
    let arr = this.shapesByLayer.get(layerId);
    if (!arr) {
      this.transact(() => {
        // Add the layer entry if missing.
        const exists = layerIndex(this.layers, layerId) >= 0;
        if (!exists) {
          const entry = new Y.Map<unknown>();
          entry.set('id', layerId);
          entry.set('name', name);
          entry.set('locked', false);
          entry.set('z_index', this.layers.length);
          this.layers.push([entry]);
        }
        arr = new Y.Array<Y.Map<unknown>>();
        this.shapesByLayer.set(layerId, arr);
      });
      arr = this.shapesByLayer.get(layerId)!;
    }
    return arr;
  }

  /** Read all layers in z-order. Client-only visibility toggles live outside the CRDT. */
  readLayers(): LayerMeta[] {
    const out: LayerMeta[] = [];
    for (const m of this.layers) {
      out.push({
        id: (m.get('id') as string) ?? '',
        name: (m.get('name') as string) ?? '',
        locked: Boolean(m.get('locked')),
        z_index: (m.get('z_index') as number) ?? 0,
      });
    }
    return out.sort((a, b) => a.z_index - b.z_index);
  }

  /** Toggle lock state on a layer. Locked layers reject edits client-side. */
  setLayerLocked(layerId: string, locked: boolean): void {
    const idx = layerIndex(this.layers, layerId);
    if (idx < 0) return;
    this.transact(() => {
      this.layers.get(idx).set('locked', locked);
    });
  }

  isLayerLocked(layerId: string): boolean {
    const idx = layerIndex(this.layers, layerId);
    if (idx < 0) return false;
    return Boolean(this.layers.get(idx).get('locked'));
  }
}

/** Find a layer's index in the layers array, or -1. */
function layerIndex(layers: Y.Array<Y.Map<unknown>>, layerId: string): number {
  for (let i = 0; i < layers.length; i++) {
    if (layers.get(i).get('id') === layerId) return i;
  }
  return -1;
}
