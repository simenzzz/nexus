<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { fetchPost, type Post } from '$lib/stores/posts';

  let postId = $derived($page.params.postId ?? '');
  let post = $state<Post | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  $effect(() => {
    if (!postId) return;
    loading = true;
    fetchPost(postId)
      .then((p) => {
        // Drafts shouldn't render here — bounce to the editor (the API
        // already gated read access, so reaching this means we're allowed).
        if (!p.published) {
          goto(`/posts/${postId}/edit`, { replaceState: true });
          return;
        }
        post = p;
        loading = false;
      })
      .catch((err) => {
        error = err instanceof Error ? err.message : String(err);
        loading = false;
      });
  });
</script>

<div class="container">
  {#if loading}
    <p>Loading...</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if post}
    <header>
      <h1>{post.title}</h1>
      <span class="badge published">Published</span>
    </header>
    {#if post.published_content !== null}
      <pre class="content">{post.published_content}</pre>
    {/if}
  {/if}
</div>

<style>
  .container {
    max-width: 48rem;
    margin: 2rem auto;
    padding: 0 1rem;
  }
  header {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1rem;
  }
  h1 {
    flex: 1;
    margin: 0;
  }
  .badge {
    padding: 0.2rem 0.5rem;
    border-radius: 9999px;
    font-size: 0.75rem;
    background: #d1fae5;
    color: #065f46;
  }
  .content {
    white-space: pre-wrap;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    padding: 1rem;
    background: #f9fafb;
    border-radius: 0.375rem;
  }
  .error {
    color: #e11d48;
  }
</style>
