<script lang="ts">
  /**
   * Overlay rendering peer cursors on top of the canvas. Awareness blobs are
   * shaped `{ cursor: {x,y}, tool, color }` by the Whiteboard component.
   */
  let { peers }: { peers: Record<string, unknown> } = $props();

  interface PeerCursor {
    userId: string;
    x: number;
    y: number;
    color: string;
    tool: string;
  }

  let cursors: PeerCursor[] = $derived(
    Object.entries(peers).flatMap(([userId, raw]) => {
      const blob = raw as { cursor?: { x: number; y: number }; color?: string; tool?: string };
      if (!blob?.cursor) return [];
      return [
        {
          userId,
          x: blob.cursor.x,
          y: blob.cursor.y,
          color: blob.color ?? '#888',
          tool: blob.tool ?? 'pen',
        },
      ];
    }),
  );
</script>

<div class="overlay" aria-hidden="true">
  {#each cursors as c (c.userId)}
    <div
      class="cursor"
      style="left:{c.x}px; top:{c.y}px; --c:{c.color}"
      title={c.userId}
    >
      <span class="dot"></span>
      <span class="label">{c.userId.slice(0, 8)}</span>
    </div>
  {/each}
</div>

<style>
  .overlay {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  .cursor {
    position: absolute;
    transform: translate(-4px, -4px);
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
  .dot {
    width: 8px;
    height: 8px;
    background: var(--c);
    border: 2px solid white;
    border-radius: 50%;
    box-shadow: 0 0 2px rgba(0, 0, 0, 0.4);
  }
  .label {
    font-size: 10px;
    color: white;
    background: var(--c);
    padding: 1px 4px;
    border-radius: 3px;
    font-family: ui-monospace, monospace;
  }
</style>
