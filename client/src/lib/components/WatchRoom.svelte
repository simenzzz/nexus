<script lang="ts">
  import WatchPlayer from './WatchPlayer.svelte';
  import WatchViewers from './WatchViewers.svelte';
  import type { WatchRoomState } from '$stores/watch';

  let {
    state,
    currentUserId,
  }: { state: WatchRoomState; currentUserId: string } = $props();

  const isLeader = $derived(state.leader_id === currentUserId);
  const hasVideo = $derived(state.playback.video_id !== null);
</script>

<div class="watch-room">
  <main class="player-pane">
    {#if hasVideo}
      <WatchPlayer
        channelId={state.channel_id}
        playback={state.playback}
        {isLeader}
      />
    {:else}
      <div class="empty">
        <p class="text-gray-400">No video playing.</p>
        <p class="text-sm text-gray-500 mt-2">
          Queue support lands in the next commit.
        </p>
      </div>
    {/if}
    {#if state.error}
      <div class="error-banner">{state.error}</div>
    {/if}
  </main>
  <WatchViewers
    channelId={state.channel_id}
    viewers={state.viewers}
    {currentUserId}
    {isLeader}
  />
</div>

<style>
  .watch-room {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .player-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 1rem;
    gap: 1rem;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: #111827;
    border-radius: 0.5rem;
  }
  .error-banner {
    background: rgb(127, 29, 29);
    color: rgb(254, 226, 226);
    padding: 0.5rem 0.75rem;
    border-radius: 0.375rem;
    font-size: 0.875rem;
  }
</style>
