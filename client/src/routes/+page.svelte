<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';

  onMount(() => {
    // TODO: Check auth state and redirect accordingly
    goto('/login');
  });
</script>

<div class="flex items-center justify-center min-h-screen">
  <p class="text-gray-400">Redirecting...</p>
</div>
