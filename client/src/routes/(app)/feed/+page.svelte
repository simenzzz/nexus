<script lang="ts">
  import PostCard from '$lib/components/PostCard.svelte';
  import { fetchPublishedPosts, postIdToString, type Post } from '$lib/stores/posts';

  let items = $state<Post[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  $effect(() => {
    loading = true;
    fetchPublishedPosts()
      .then((p) => {
        items = p;
        loading = false;
      })
      .catch((err) => {
        error = err instanceof Error ? err.message : String(err);
        loading = false;
      });
  });
</script>

<div class="container">
  <h1>Feed</h1>

  {#if loading}
    <p>Loading...</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if items.length === 0}
    <p class="empty">No published posts yet. <a href="/posts/new">Write one</a>.</p>
  {:else}
    <ul class="posts">
      {#each items as post (postIdToString(post))}
        <li>
          <PostCard {post} />
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .container {
    max-width: 48rem;
    margin: 2rem auto;
    padding: 0 1rem;
  }
  h1 {
    margin-bottom: 1rem;
  }
  .posts {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .error {
    color: #e11d48;
  }
  .empty {
    color: #6b7280;
  }
</style>
