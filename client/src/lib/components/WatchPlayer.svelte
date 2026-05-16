<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { createYouTubePlayer, type YouTubePlayer } from '$lib/watch/youtube-player';
  import { createSyncController, type SyncController } from '$lib/watch/sync-controller';
  import { sendPlayback, type WatchPlayback } from '$stores/watch';

  let {
    channelId,
    playback,
    isLeader,
  }: { channelId: string; playback: WatchPlayback; isLeader: boolean } = $props();

  let mountEl: HTMLDivElement;
  let player: YouTubePlayer | null = null;
  let sync: SyncController | null = null;
  let lastApplyKey = '';

  // Apply transitions whenever the authoritative playback changes. Use a
  // string key (action + ts + video) so we don't re-apply identical updates
  // and accidentally yank the leader's seek bar.
  $effect(() => {
    const key = `${playback.video_id ?? ''}|${playback.server_ts}|${playback.paused}|${playback.position_ms}`;
    if (!sync || key === lastApplyKey) return;
    lastApplyKey = key;
    sync.apply(playback, isLeader);
  });

  // Reconcile on every pulse — the `server_ts` updates each tick. The
  // controller decides whether to actually correct.
  $effect(() => {
    if (!sync) return;
    sync.reconcile(playback, isLeader);
  });

  onMount(async () => {
    player = createYouTubePlayer(mountEl);
    await player.ready;
    sync = createSyncController(player);
    // Initial hydrate.
    sync.apply(playback, isLeader);

    // The leader's local player events flow upstream to the server. Followers
    // never emit (their player is locked via controls=0 anyway).
    player.on((e) => {
      if (!isLeader) return;
      switch (e.kind) {
        case 'play':
          sendPlayback(channelId, 'play', player!.getPosition());
          break;
        case 'pause':
          sendPlayback(channelId, 'pause', player!.getPosition());
          break;
        case 'seek':
          sendPlayback(channelId, 'seek', e.position_ms);
          break;
      }
    });
  });

  onDestroy(() => {
    sync?.stop();
    player?.destroy();
    player = null;
    sync = null;
  });
</script>

<div class="watch-player-shell">
  <div class="iframe-mount" bind:this={mountEl}></div>
  {#if !isLeader}
    <div class="follower-overlay" title="Only the leader can control playback"></div>
  {/if}
</div>

<style>
  .watch-player-shell {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 9;
    background: #000;
  }
  .iframe-mount {
    width: 100%;
    height: 100%;
  }
  /* For followers, swallow pointer events so they can't try to use the
     native iframe controls (which YouTube exposes intermittently even with
     controls=0). */
  .follower-overlay {
    position: absolute;
    inset: 0;
    pointer-events: auto;
    cursor: not-allowed;
    background: transparent;
  }
</style>
