<script lang="ts">
  import type { WhiteboardProvider, LayerMeta } from '$lib/collab/whiteboard-provider';
  import { layerVisibility, toolState } from '$lib/stores/whiteboards';

  let { provider }: { provider: WhiteboardProvider | null } = $props();

  let layers: LayerMeta[] = $state([]);
  let visibility: Record<string, boolean> = $state({});
  let activeId = $state('default');

  // Re-read layers whenever the underlying Y.Array changes.
  $effect(() => {
    if (!provider) return;
    const refresh = () => {
      layers = provider!.readLayers();
    };
    refresh();
    provider.layers.observeDeep(refresh);
    return () => provider!.layers.unobserveDeep(refresh);
  });

  layerVisibility.subscribe((v) => (visibility = v));
  toolState.subscribe((s) => (activeId = s.activeLayerId));

  function toggleVisible(id: string) {
    layerVisibility.update((v) => ({ ...v, [id]: !(v[id] !== false) }));
  }

  function toggleLock(layer: LayerMeta) {
    provider?.setLayerLocked(layer.id, !layer.locked);
  }

  function setActive(id: string) {
    toolState.update((s) => ({ ...s, activeLayerId: id }));
  }

  function addLayer() {
    if (!provider) return;
    const id = `layer-${Math.random().toString(36).slice(2, 8)}`;
    const name = prompt('Layer name:', 'New layer') ?? 'New layer';
    provider.ensureLayer(id, name);
    setActive(id);
  }
</script>

<aside class="layers">
  <div class="header">
    <span>Layers</span>
    <button type="button" onclick={addLayer}>+</button>
  </div>
  <ul>
    {#each layers as layer (layer.id)}
      <li class:active={layer.id === activeId}>
        <button class="row" type="button" onclick={() => setActive(layer.id)}>
          <span class="name">{layer.name}</span>
        </button>
        <button
          type="button"
          class="icon"
          title={visibility[layer.id] === false ? 'Show' : 'Hide'}
          onclick={() => toggleVisible(layer.id)}
        >
          {visibility[layer.id] === false ? '○' : '●'}
        </button>
        <button
          type="button"
          class="icon"
          title={layer.locked ? 'Unlock' : 'Lock'}
          onclick={() => toggleLock(layer)}
        >
          {layer.locked ? '🔒' : '🔓'}
        </button>
      </li>
    {/each}
  </ul>
</aside>

<style>
  .layers {
    width: 200px;
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 4px;
    padding: 0.5rem;
    font-size: 0.85rem;
  }
  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.5rem;
    font-weight: 600;
  }
  .header button {
    padding: 0.1rem 0.5rem;
    font-size: 1rem;
    line-height: 1;
    cursor: pointer;
    border: 1px solid #ccc;
    background: #fff;
    border-radius: 4px;
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  li {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.2rem;
    border-radius: 4px;
  }
  li.active {
    background: #dbeafe;
  }
  .row {
    flex: 1;
    text-align: left;
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 0.15rem 0.3rem;
  }
  .icon {
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 0.15rem 0.25rem;
  }
</style>
