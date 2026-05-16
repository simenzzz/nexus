<script lang="ts">
  import { toolState, type ToolKind } from '$lib/stores/whiteboards';

  const TOOLS: { kind: ToolKind; label: string }[] = [
    { kind: 'pen', label: 'Pen' },
    { kind: 'rect', label: 'Rect' },
    { kind: 'circle', label: 'Circle' },
    { kind: 'line', label: 'Line' },
    { kind: 'arrow', label: 'Arrow' },
    { kind: 'text', label: 'Text' },
    { kind: 'eraser', label: 'Eraser' },
    { kind: 'select', label: 'Select' },
  ];

  let current: ToolKind = $state('pen');
  let color = $state('#222222');
  let width = $state(3);

  toolState.subscribe((s) => {
    current = s.tool;
    color = s.color;
    width = s.strokeWidth;
  });

  function setTool(kind: ToolKind) {
    toolState.update((s) => ({ ...s, tool: kind }));
  }
  function setColor(c: string) {
    toolState.update((s) => ({ ...s, color: c }));
  }
  function setWidth(w: number) {
    toolState.update((s) => ({ ...s, strokeWidth: w }));
  }
</script>

<div class="toolbar">
  {#each TOOLS as t}
    <button
      class:active={current === t.kind}
      onclick={() => setTool(t.kind)}
      type="button"
    >
      {t.label}
    </button>
  {/each}

  <label class="color">
    Color
    <input type="color" value={color} oninput={(e) => setColor(e.currentTarget.value)} />
  </label>

  <label class="width">
    Width
    <input
      type="range"
      min="1"
      max="20"
      value={width}
      oninput={(e) => setWidth(Number(e.currentTarget.value))}
    />
    <span>{width}px</span>
  </label>
</div>

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    padding: 0.5rem;
    background: #f5f5f5;
    border-radius: 4px;
  }
  button {
    padding: 0.35rem 0.75rem;
    border: 1px solid #ccc;
    background: #fff;
    cursor: pointer;
    border-radius: 4px;
    font-size: 0.85rem;
  }
  button.active {
    background: #2563eb;
    color: white;
    border-color: #2563eb;
  }
  .color,
  .width {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
    color: #555;
  }
  .color input {
    width: 32px;
    height: 24px;
    border: 1px solid #ccc;
    cursor: pointer;
  }
  .width span {
    min-width: 2.5rem;
    text-align: right;
  }
</style>
