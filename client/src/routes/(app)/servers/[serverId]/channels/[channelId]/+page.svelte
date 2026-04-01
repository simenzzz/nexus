<script lang="ts">
  import MessageList from '$components/MessageList.svelte';
  import ChatInput from '$components/ChatInput.svelte';
  import ChannelList from '$components/ChannelList.svelte';
</script>

<div class="flex h-full">
  <ChannelList />
  <div class="flex flex-col flex-1">
    <div class="flex-1 overflow-y-auto p-4">
      <MessageList />
    </div>
    <ChatInput />
  </div>
</div>
