<script lang="ts">
  import ServerSidebar from '$components/ServerSidebar.svelte';
  let { children } = $props();
</script>

<div class="flex h-screen">
  <ServerSidebar />
  <main class="flex-1 overflow-y-auto">
    {@render children()}
  </main>
</div>
