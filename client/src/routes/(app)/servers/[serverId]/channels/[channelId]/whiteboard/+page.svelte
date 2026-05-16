<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import ChannelList from '$components/ChannelList.svelte';
  import DrawingTools from '$components/DrawingTools.svelte';
  import Whiteboard from '$components/Whiteboard.svelte';
  import WhiteboardLayer from '$components/WhiteboardLayer.svelte';
  import { fetchChannels } from '$stores/channels';
  import type { WhiteboardProvider } from '$lib/collab/whiteboard-provider';
  import {
    createCheckpoint,
    fetchWhiteboard,
    listCheckpoints,
    restoreCheckpoint,
    type WhiteboardCheckpoint,
    type WhiteboardSnapshot,
    checkpointIdToString,
  } from '$stores/whiteboards';

  const serverId = $derived(page.params.serverId ?? '');
  const channelId = $derived(page.params.channelId ?? '');

  let snapshot: WhiteboardSnapshot | null = $state(null);
  let checkpoints: WhiteboardCheckpoint[] = $state([]);
  let provider: WhiteboardProvider | null = $state(null);
  let error = $state('');
  let busy = $state(false);

  onMount(async () => {
    if (!serverId || !channelId) return;
    await fetchChannels(serverId);
    try {
      snapshot = await fetchWhiteboard(channelId);
      checkpoints = await listCheckpoints(channelId);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  });

  async function onSaveVersion() {
    if (!channelId) return;
    busy = true;
    try {
      const label = prompt('Label this version (optional):') ?? undefined;
      await createCheckpoint(channelId, label);
      checkpoints = await listCheckpoints(channelId);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function onRestore(cp: WhiteboardCheckpoint) {
    if (!channelId) return;
    if (!confirm(`Restore "${cp.label ?? 'snapshot'}"? Current state will be overwritten.`))
      return;
    busy = true;
    try {
      await restoreCheckpoint(channelId, checkpointIdToString(cp));
      // WhiteboardClosed will fire; provider re-subscribes and pulls fresh.
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="flex h-full">
  <ChannelList {serverId} />
  <div class="flex flex-col flex-1 p-4 gap-3 overflow-auto">
    {#if error}
      <div class="bg-red-100 text-red-800 p-2 rounded text-sm">{error}</div>
    {/if}

    <DrawingTools />

    <div class="flex gap-3 items-start">
      {#if snapshot}
        <Whiteboard
          {channelId}
          initialStateB64={snapshot.state_b64}
          onReady={(p) => (provider = p)}
        />
      {:else}
        <div class="text-gray-500 text-sm">Loading whiteboard…</div>
      {/if}

      <div class="flex flex-col gap-3">
        <WhiteboardLayer {provider} />

        <div class="checkpoints bg-gray-50 border border-gray-200 rounded p-2 text-sm w-48">
          <div class="flex justify-between items-center mb-2 font-semibold">
            <span>Versions</span>
            <button
              type="button"
              class="px-2 py-0.5 bg-blue-600 text-white rounded text-xs disabled:opacity-50"
              onclick={onSaveVersion}
              disabled={busy}
            >
              Save
            </button>
          </div>
          <ul class="flex flex-col gap-1 max-h-64 overflow-auto">
            {#each checkpoints as cp (checkpointIdToString(cp))}
              <li class="flex items-center justify-between gap-2">
                <span class="truncate" title={cp.label ?? ''}>
                  {cp.label ?? 'snapshot'}
                </span>
                <button
                  type="button"
                  class="text-xs text-blue-700 hover:underline disabled:opacity-50"
                  onclick={() => onRestore(cp)}
                  disabled={busy}
                >
                  restore
                </button>
              </li>
            {:else}
              <li class="text-gray-500 text-xs">No saved versions</li>
            {/each}
          </ul>
        </div>
      </div>
    </div>
  </div>
</div>

