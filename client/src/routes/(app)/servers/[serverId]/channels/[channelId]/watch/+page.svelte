<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { page } from '$app/state';
  import ChannelList from '$components/ChannelList.svelte';
  import WatchRoom from '$components/WatchRoom.svelte';
  import { auth } from '$stores/auth';
  import { fetchChannels } from '$stores/channels';
  import { bindWatchRoom, watchRoomStore } from '$stores/watch';

  const serverId = $derived(page.params.serverId ?? '');
  const channelId = $derived(page.params.channelId ?? '');

  // Reactively re-bind when the channelId changes (e.g. nav between rooms).
  let unbind: (() => void) | null = null;
  const room = $derived(channelId ? watchRoomStore(channelId) : null);

  onMount(async () => {
    if (serverId) {
      await fetchChannels(serverId);
    }
  });

  $effect(() => {
    unbind?.();
    unbind = null;
    if (channelId) {
      unbind = bindWatchRoom(channelId);
    }
  });

  onDestroy(() => {
    unbind?.();
  });
</script>

<div class="flex h-full">
  <ChannelList {serverId} />
  <div class="flex flex-col flex-1 min-w-0">
    {#if room && $room && $auth.user}
      <WatchRoom state={$room} currentUserId={$auth.user.id} />
    {:else}
      <div class="flex-1 flex items-center justify-center text-gray-500">
        Loading watch room…
      </div>
    {/if}
  </div>
</div>
