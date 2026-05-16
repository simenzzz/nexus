<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import * as Y from 'yjs';
  import {
    DEFAULT_LAYER_ID,
    WhiteboardProvider,
    type LayerMeta,
  } from '$lib/collab/whiteboard-provider';
  import { simplify, type Point } from '$lib/canvas/rdp';
  import {
    layerVisibility,
    toolState,
    type ToolKind,
  } from '$lib/stores/whiteboards';
  import WhiteboardCursors from './WhiteboardCursors.svelte';

  let {
    channelId,
    initialStateB64,
    width = 1600,
    height = 1000,
    onReady,
  }: {
    channelId: string;
    initialStateB64?: string;
    width?: number;
    height?: number;
    /** Called once the provider is constructed so the parent can wire side
     * panels (layers, peer list, etc.) against the same Y.Doc. */
    onReady?: (provider: WhiteboardProvider) => void;
  } = $props();

  let canvas: HTMLCanvasElement;
  let provider: WhiteboardProvider | null = $state(null);
  let layers: LayerMeta[] = $state([]);
  let visibility: Record<string, boolean> = $state({});
  let tool: ToolKind = $state('pen');
  let color = $state('#222222');
  let strokeWidth = $state(3);
  let activeLayerId = $state(DEFAULT_LAYER_ID);
  let peers: Record<string, unknown> = $state({});

  // In-flight stroke / drag state. Kept local so we can stream incrementally.
  let drawingShape: Y.Map<unknown> | null = null;
  let drawingPoints: Y.Array<{ x: number; y: number }> | null = null;
  let rawPenPoints: Point[] = [];
  let dragStart: Point | null = null;
  let movingShape: Y.Map<unknown> | null = null;
  let moveOrigin: Point | null = null;

  let rafScheduled = false;

  /**
   * Subscriptions tied to a single provider instance. Returns a tear-down
   * function that detaches everything. Called both on first mount and after
   * a checkpoint restore (`onClosed`) where we destroy + rebuild the
   * provider — without grouping the teardown here, the restore path leaked
   * Y.Doc update listeners and observeDeep handlers on every restore.
   */
  function attachProviderListeners(p: WhiteboardProvider): () => void {
    const onUpdate = () => requestRepaint();
    p.doc.on('update', onUpdate);

    const onLayersChange = () => {
      refreshLayers();
      requestRepaint();
    };
    p.layers.observeDeep(onLayersChange);

    const offAw = p.onAwareness((users) => {
      peers = users;
    });

    return () => {
      p.doc.off('update', onUpdate);
      p.layers.unobserveDeep(onLayersChange);
      offAw();
    };
  }

  // Active per-provider teardown. Replaced when restore rebuilds the provider.
  let detachProvider: (() => void) | null = null;

  onMount(() => {
    provider = new WhiteboardProvider(channelId);

    // Seed from REST snapshot so the canvas paints something before the
    // server's `whiteboard_state` message arrives over WS.
    if (initialStateB64) {
      try {
        const bytes = base64ToBytes(initialStateB64);
        Y.applyUpdate(provider.doc, bytes);
      } catch (e) {
        console.error('Failed to hydrate from REST snapshot', e);
      }
    }

    provider.ensureLayer(DEFAULT_LAYER_ID, 'Default');
    refreshLayers();
    onReady?.(provider);
    detachProvider = attachProviderListeners(provider);

    const offTool = toolState.subscribe((s) => {
      tool = s.tool;
      color = s.color;
      strokeWidth = s.strokeWidth;
      activeLayerId = s.activeLayerId;
    });

    const offVis = layerVisibility.subscribe((v) => {
      visibility = v;
      requestRepaint();
    });

    const offClosed = provider.onClosed(() => {
      // Server tore down the session (e.g. checkpoint restore). Detach old
      // listeners explicitly so we don't leak update/observeDeep handlers,
      // then rebuild the provider and re-attach.
      detachProvider?.();
      detachProvider = null;
      provider?.destroy();
      peers = {}; // stale peer cursors from the old session shouldn't linger
      provider = new WhiteboardProvider(channelId);
      provider.ensureLayer(DEFAULT_LAYER_ID, 'Default');
      refreshLayers();
      onReady?.(provider);
      detachProvider = attachProviderListeners(provider);
      requestRepaint();
    });

    // Fallback for pointer-up that happens off-canvas (drag past window edge,
    // window blur). Without this, a stroke started inside the canvas but
    // released outside would never call our handler.
    const onWindowPointerUp = () => finishStroke(null);
    window.addEventListener('pointerup', onWindowPointerUp);
    window.addEventListener('blur', onWindowPointerUp);

    requestRepaint();

    return () => {
      window.removeEventListener('pointerup', onWindowPointerUp);
      window.removeEventListener('blur', onWindowPointerUp);
      detachProvider?.();
      detachProvider = null;
      offTool();
      offVis();
      offClosed();
    };
  });

  onDestroy(() => {
    provider?.destroy();
    provider = null;
  });

  function refreshLayers() {
    if (!provider) return;
    layers = provider.readLayers();
    // Initialize visibility for any newly-seen layer.
    layerVisibility.update((v) => {
      const next = { ...v };
      for (const l of layers) {
        if (!(l.id in next)) next[l.id] = true;
      }
      return next;
    });
  }

  function requestRepaint() {
    if (rafScheduled) return;
    rafScheduled = true;
    requestAnimationFrame(() => {
      rafScheduled = false;
      paint();
    });
  }

  function paint() {
    if (!canvas || !provider) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    for (const layer of layers) {
      if (visibility[layer.id] === false) continue;
      const shapes = provider.shapesByLayer.get(layer.id);
      if (!shapes) continue;
      for (let i = 0; i < shapes.length; i++) {
        const shape = shapes.get(i);
        if (shape.get('deleted')) continue;
        drawShape(ctx, shape);
      }
    }
  }

  function drawShape(ctx: CanvasRenderingContext2D, shape: Y.Map<unknown>) {
    const type = shape.get('type') as string;
    const c = (shape.get('color') as string) ?? '#000';
    const w = (shape.get('width') as number) ?? 2;
    const pts = readPoints(shape);

    ctx.strokeStyle = c;
    ctx.fillStyle = c;
    ctx.lineWidth = w;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';

    if (type === 'pen' && pts.length > 0) {
      ctx.beginPath();
      ctx.moveTo(pts[0].x, pts[0].y);
      for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x, pts[i].y);
      ctx.stroke();
    } else if (type === 'rect' && pts.length === 2) {
      const [a, b] = pts;
      ctx.strokeRect(a.x, a.y, b.x - a.x, b.y - a.y);
    } else if (type === 'circle' && pts.length === 2) {
      const [a, b] = pts;
      const r = Math.hypot(b.x - a.x, b.y - a.y);
      ctx.beginPath();
      ctx.arc(a.x, a.y, r, 0, Math.PI * 2);
      ctx.stroke();
    } else if (type === 'line' && pts.length === 2) {
      const [a, b] = pts;
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.stroke();
    } else if (type === 'arrow' && pts.length === 2) {
      const [a, b] = pts;
      const head = 10 + w * 2;
      const angle = Math.atan2(b.y - a.y, b.x - a.x);
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.lineTo(
        b.x - head * Math.cos(angle - Math.PI / 6),
        b.y - head * Math.sin(angle - Math.PI / 6),
      );
      ctx.moveTo(b.x, b.y);
      ctx.lineTo(
        b.x - head * Math.cos(angle + Math.PI / 6),
        b.y - head * Math.sin(angle + Math.PI / 6),
      );
      ctx.stroke();
    } else if (type === 'text' && pts.length >= 1) {
      const txt = (shape.get('text') as string) ?? '';
      ctx.font = `${12 + w * 3}px sans-serif`;
      ctx.fillText(txt, pts[0].x, pts[0].y);
    }
  }

  function readPoints(shape: Y.Map<unknown>): Point[] {
    const arr = shape.get('points') as Y.Array<{ x: number; y: number }> | undefined;
    if (!arr) return [];
    const out: Point[] = [];
    for (let i = 0; i < arr.length; i++) {
      const p = arr.get(i);
      out.push({ x: p.x, y: p.y });
    }
    return out;
  }

  function eventPoint(e: PointerEvent): Point {
    const rect = canvas.getBoundingClientRect();
    // Scale to canvas resolution in case the canvas is displayed smaller.
    const sx = canvas.width / rect.width;
    const sy = canvas.height / rect.height;
    return {
      x: (e.clientX - rect.left) * sx,
      y: (e.clientY - rect.top) * sy,
    };
  }

  function onPointerDown(e: PointerEvent) {
    if (!provider || provider.isLayerLocked(activeLayerId)) return;
    canvas.setPointerCapture(e.pointerId);
    const p = eventPoint(e);

    if (tool === 'eraser') {
      eraseAt(p);
      return;
    }
    if (tool === 'select') {
      const hit = hitTest(p);
      if (hit) {
        movingShape = hit;
        moveOrigin = p;
      }
      return;
    }
    if (tool === 'text') {
      const txt = prompt('Text:') ?? '';
      if (txt.trim()) {
        provider.transact(() => {
          createShape('text', [p], { text: txt });
        });
      }
      return;
    }

    // Pen / rect / circle / line / arrow — start a streaming shape.
    dragStart = p;
    rawPenPoints = tool === 'pen' ? [p] : [];
    provider.transact(() => {
      const shape = createShape(tool, tool === 'pen' ? [p] : [p, p]);
      drawingShape = shape;
      drawingPoints = shape.get('points') as Y.Array<{ x: number; y: number }>;
    });
  }

  function onPointerMove(e: PointerEvent) {
    if (!provider) return;
    const p = eventPoint(e);

    // Awareness: broadcast cursor at ≤30 Hz throttle (server caps at 30/s).
    sendCursorThrottled(p);

    if (movingShape && moveOrigin) {
      const dx = p.x - moveOrigin.x;
      const dy = p.y - moveOrigin.y;
      const pts = movingShape.get('points') as Y.Array<{ x: number; y: number }>;
      provider.transact(() => {
        const len = pts.length;
        const moved: { x: number; y: number }[] = [];
        for (let i = 0; i < len; i++) {
          const old = pts.get(i);
          moved.push({ x: old.x + dx, y: old.y + dy });
        }
        pts.delete(0, len);
        pts.insert(0, moved);
      });
      moveOrigin = p;
      return;
    }

    if (!drawingShape || !drawingPoints) return;

    if (tool === 'pen') {
      rawPenPoints.push(p);
      provider.transact(() => {
        drawingPoints!.push([{ x: p.x, y: p.y }]);
      });
    } else if (dragStart) {
      // Update the end-point of the shape (start point stays at index 0).
      provider.transact(() => {
        drawingPoints!.delete(1, drawingPoints!.length - 1);
        drawingPoints!.push([{ x: p.x, y: p.y }]);
      });
    }
  }

  function onPointerUp(e: PointerEvent) {
    finishStroke(e.pointerId);
  }

  /**
   * Pointer interruption (browser drop, focus loss, touch interrupted by a
   * system notification) — without this, `drawingShape`/`drawingPoints` stay
   * set after the pointer goes away, and the next pointerdown starts a
   * *second* stroke while the first dangles uncompressed forever. The
   * fallback `window` pointerup listener (wired in onMount) covers the case
   * where the pointer goes up *outside* the canvas element entirely.
   */
  function onPointerCancel(e: PointerEvent) {
    finishStroke(e.pointerId);
  }

  function finishStroke(pointerId: number | null) {
    if (!provider) return;
    if (pointerId !== null) {
      try {
        canvas.releasePointerCapture(pointerId);
      } catch {
        // Already released — safe to ignore.
      }
    }
    movingShape = null;
    moveOrigin = null;

    if (drawingShape && drawingPoints && tool === 'pen' && rawPenPoints.length > 2) {
      // Replace raw points with RDP-compressed version. This is one
      // transaction so peers see a single canonical update.
      const compressed = simplify(rawPenPoints, 1);
      provider.transact(() => {
        drawingPoints!.delete(0, drawingPoints!.length);
        drawingPoints!.insert(
          0,
          compressed.map((p) => ({ x: p.x, y: p.y })),
        );
      });
    }
    drawingShape = null;
    drawingPoints = null;
    dragStart = null;
    rawPenPoints = [];
  }

  function createShape(
    kind: ToolKind,
    points: Point[],
    extra: Record<string, unknown> = {},
  ): Y.Map<unknown> {
    if (!provider) throw new Error('not initialized');
    const shapes = provider.ensureLayer(activeLayerId);
    const shape = new Y.Map<unknown>();
    shape.set('type', kind);
    shape.set('color', color);
    shape.set('width', strokeWidth);
    shape.set('layer_id', activeLayerId);
    shape.set('created_at', Date.now());
    shape.set('deleted', false);
    const arr = new Y.Array<{ x: number; y: number }>();
    arr.insert(
      0,
      points.map((p) => ({ x: p.x, y: p.y })),
    );
    shape.set('points', arr);
    for (const [k, v] of Object.entries(extra)) shape.set(k, v);
    shapes.push([shape]);
    return shape;
  }

  function eraseAt(p: Point) {
    const hit = hitTest(p);
    if (!hit || !provider) return;
    provider.transact(() => {
      hit.set('deleted', true);
    });
  }

  /** Simple bbox hit test, walking shapes top-most layer first. */
  function hitTest(p: Point): Y.Map<unknown> | null {
    if (!provider) return null;
    for (let li = layers.length - 1; li >= 0; li--) {
      const layer = layers[li];
      if (visibility[layer.id] === false || layer.locked) continue;
      const shapes = provider.shapesByLayer.get(layer.id);
      if (!shapes) continue;
      for (let i = shapes.length - 1; i >= 0; i--) {
        const s = shapes.get(i);
        if (s.get('deleted')) continue;
        if (bboxContains(s, p, 6)) return s;
      }
    }
    return null;
  }

  function bboxContains(shape: Y.Map<unknown>, p: Point, slop: number): boolean {
    const pts = readPoints(shape);
    if (pts.length === 0) return false;
    let minX = pts[0].x;
    let minY = pts[0].y;
    let maxX = pts[0].x;
    let maxY = pts[0].y;
    for (const q of pts) {
      if (q.x < minX) minX = q.x;
      if (q.y < minY) minY = q.y;
      if (q.x > maxX) maxX = q.x;
      if (q.y > maxY) maxY = q.y;
    }
    return p.x >= minX - slop && p.x <= maxX + slop && p.y >= minY - slop && p.y <= maxY + slop;
  }

  // Throttle awareness to ≤30 Hz (server-side cap).
  let lastAwarenessAt = 0;
  function sendCursorThrottled(p: Point) {
    const now = performance.now();
    if (now - lastAwarenessAt < 35) return;
    lastAwarenessAt = now;
    provider?.sendAwareness({ cursor: p, tool, color });
  }

  function base64ToBytes(b64: string): Uint8Array {
    const s = atob(b64);
    const bytes = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
    return bytes;
  }
</script>

<div class="whiteboard">
  <canvas
    bind:this={canvas}
    {width}
    {height}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerCancel}
  ></canvas>
  <WhiteboardCursors {peers} />
</div>

<style>
  .whiteboard {
    position: relative;
    display: inline-block;
    background: #fff;
    border: 1px solid #ddd;
    border-radius: 4px;
    cursor: crosshair;
  }
  canvas {
    display: block;
    max-width: 100%;
    height: auto;
    touch-action: none;
  }
</style>
