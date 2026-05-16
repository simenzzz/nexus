<script lang="ts">
  import { inviteCollaborator } from '$lib/stores/posts';

  let { postId }: { postId: string } = $props();

  let userId = $state('');
  let pending = $state(false);
  let error = $state<string | null>(null);
  let success = $state<string | null>(null);

  async function onSubmit(e: Event) {
    e.preventDefault();
    const trimmed = userId.trim();
    if (!trimmed) {
      error = 'User id required';
      success = null;
      return;
    }
    pending = true;
    error = null;
    success = null;
    try {
      await inviteCollaborator(postId, trimmed);
      success = `Invited ${trimmed}`;
      userId = '';
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      pending = false;
    }
  }
</script>

<form onsubmit={onSubmit} class="invite">
  <label for="invite-user">Invite collaborator</label>
  <div class="row">
    <input
      id="invite-user"
      type="text"
      bind:value={userId}
      placeholder="user id"
      disabled={pending}
    />
    <button type="submit" disabled={pending || !userId.trim()}>
      {pending ? 'Inviting…' : 'Invite'}
    </button>
  </div>
  {#if error}
    <p class="error">{error}</p>
  {:else if success}
    <p class="success">{success}</p>
  {/if}
</form>

<style>
  .invite {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.75rem;
    border: 1px solid var(--color-border, #e5e7eb);
    border-radius: 0.4rem;
    background: #fafafa;
  }
  label {
    font-size: 0.85rem;
    color: #374151;
    font-weight: 500;
  }
  .row {
    display: flex;
    gap: 0.5rem;
  }
  input {
    flex: 1;
    padding: 0.4rem 0.6rem;
    border: 1px solid #d1d5db;
    border-radius: 0.25rem;
  }
  button {
    padding: 0.4rem 0.9rem;
    border: none;
    border-radius: 0.25rem;
    background: #2563eb;
    color: white;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .error {
    color: #e11d48;
    margin: 0;
    font-size: 0.85rem;
  }
  .success {
    color: #047857;
    margin: 0;
    font-size: 0.85rem;
  }
</style>
