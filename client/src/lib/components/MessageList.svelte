<script lang="ts">
  import { chat } from '$stores/chat';
</script>

<div class="space-y-2">
  {#each $chat.messages as message (message.id)}
    <div class="flex gap-3 p-2 hover:bg-gray-800 rounded">
      <div class="font-semibold text-sm text-indigo-400">{message.authorId}</div>
      <div class="text-sm">{message.content}</div>
    </div>
  {:else}
    <p class="text-gray-500 text-center py-8">No messages yet. Start the conversation!</p>
  {/each}
</div>
