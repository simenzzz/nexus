<script lang="ts">
  import { page } from '$app/state';
  import { channels } from '$stores/channels';

  let { serverId }: { serverId: string } = $props();

  const serverChannels = $derived($channels.get(serverId) ?? []);

  function getChannelId(channel: { id: unknown }): string {
    if (!channel.id) return '';
    if (typeof channel.id === 'string') return channel.id;
    const obj = channel.id as Record<string, unknown>;
    if (obj.id && typeof obj.id === 'string') return obj.id;
    if (obj.id && typeof obj.id === 'object') {
      return String(Object.values(obj.id as Record<string, unknown>)[0] ?? '');
    }
    return '';
  }

  function channelIcon(kind: string): string {
    switch (kind) {
      case 'whiteboard':
        return '✎';
      case 'voice':
        return '🔊';
      case 'collab':
        return '✦';
      default:
        return '#';
    }
  }

  function channelHref(serverId: string, channel: { channel_type: string; id: unknown }): string {
    const id = getChannelId(channel);
    if (channel.channel_type === 'whiteboard') {
      return `/servers/${serverId}/channels/${id}/whiteboard`;
    }
    return `/servers/${serverId}/channels/${id}`;
  }
</script>

<aside class="w-60 bg-gray-800 flex flex-col shrink-0">
  <div class="p-4 font-semibold border-b border-gray-700">
    Channels
  </div>
  <div class="flex-1 overflow-y-auto p-2">
    <p class="text-xs font-semibold text-gray-400 uppercase px-2 mb-1">Channels</p>
    {#each serverChannels as channel (getChannelId(channel))}
      <a
        href={channelHref(serverId, channel)}
        class="px-2 py-1 rounded hover:bg-gray-700 cursor-pointer text-gray-300 block"
      >
        <span class="inline-block w-5 text-center">{channelIcon(channel.channel_type)}</span>
        {channel.name}
      </a>
    {:else}
      <p class="text-gray-500 text-sm px-2">No channels</p>
    {/each}
  </div>
</aside>
