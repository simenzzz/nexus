<script lang="ts">
  import { sendTransferLeader, type WatchViewer } from '$stores/watch';

  let {
    channelId,
    viewers,
    currentUserId,
    isLeader,
  }: {
    channelId: string;
    viewers: WatchViewer[];
    currentUserId: string;
    isLeader: boolean;
  } = $props();

  function handleTransfer(toUserId: string): void {
    if (!isLeader || toUserId === currentUserId) return;
    sendTransferLeader(channelId, toUserId);
  }
</script>

<aside class="viewer-list">
  <p class="text-xs font-semibold text-gray-400 uppercase mb-2">
    Viewers ({viewers.length})
  </p>
  <ul class="space-y-1">
    {#each viewers as v (v.user_id)}
      <li class="flex items-center gap-2 px-2 py-1 rounded hover:bg-gray-700">
        <span class="flex-1 text-sm text-gray-200">
          {v.username}
          {#if v.user_id === currentUserId}
            <span class="text-xs text-gray-500">(you)</span>
          {/if}
        </span>
        {#if v.is_leader}
          <span
            class="text-xs px-1.5 py-0.5 rounded bg-amber-700 text-amber-100"
            title="Room leader"
          >
            ★
          </span>
        {:else if isLeader}
          <button
            class="text-xs px-1.5 py-0.5 rounded border border-gray-600 hover:bg-gray-700"
            onclick={() => handleTransfer(v.user_id)}
            title="Transfer leadership"
          >
            Promote
          </button>
        {/if}
      </li>
    {/each}
  </ul>
</aside>

<style>
  .viewer-list {
    width: 200px;
    border-left: 1px solid rgb(55, 65, 81);
    padding: 0.5rem;
    overflow-y: auto;
  }
</style>
