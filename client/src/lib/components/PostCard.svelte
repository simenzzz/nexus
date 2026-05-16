<script lang="ts">
  import { postIdToString, type Post } from '$lib/stores/posts';

  let { post }: { post: Post } = $props();

  function preview(content: string | null): string {
    if (!content) return '';
    return content.length > 240 ? content.slice(0, 240) + '…' : content;
  }
</script>

<a href={`/posts/${postIdToString(post)}`} class="card">
  <h2>{post.title}</h2>
  <p>{preview(post.published_content)}</p>
</a>

<style>
  .card {
    display: block;
    padding: 1rem;
    border: 1px solid var(--color-border, #e5e7eb);
    border-radius: 0.5rem;
    text-decoration: none;
    color: inherit;
  }
  .card:hover {
    background: #f9fafb;
  }
  .card h2 {
    margin: 0 0 0.4rem;
    font-size: 1.1rem;
  }
  .card p {
    margin: 0;
    color: #6b7280;
    white-space: pre-wrap;
  }
</style>
