<script lang="ts">
  import FriendSuggestions from '$components/FriendSuggestions.svelte';
</script>

<div class="p-6">
  <h1 class="text-2xl font-bold mb-6">Friends</h1>

  <section class="mb-8">
    <h2 class="text-lg font-semibold text-gray-300 mb-3">Online</h2>
    <p class="text-gray-500 text-sm">No friends online.</p>
  </section>

  <section class="mb-8">
    <h2 class="text-lg font-semibold text-gray-300 mb-3">All Friends</h2>
    <p class="text-gray-500 text-sm">Your friends list will appear here.</p>
  </section>

  <section class="mb-8">
    <h2 class="text-lg font-semibold text-gray-300 mb-3">Pending Requests</h2>
    <p class="text-gray-500 text-sm">No pending friend requests.</p>
  </section>

  <FriendSuggestions />
</div>
