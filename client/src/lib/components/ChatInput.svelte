<script lang="ts">
  let message = $state('');

  function handleSubmit(e: Event) {
    e.preventDefault();
    if (!message.trim()) return;
    // TODO: Send message via WebSocket
    message = '';
  }
</script>

<form onsubmit={handleSubmit} class="p-4 border-t border-gray-700">
  <input
    type="text"
    bind:value={message}
    placeholder="Type a message..."
    class="w-full p-3 bg-gray-700 rounded-lg focus:outline-none focus:ring-2 focus:ring-indigo-500"
  />
</form>
