<script lang="ts">
  let username = $state('');
  let password = $state('');

  function handleSubmit(e: Event) {
    e.preventDefault();
    // TODO: Call API login endpoint, store JWT
  }
</script>

<div class="flex items-center justify-center min-h-screen">
  <div class="w-full max-w-md p-8 bg-gray-800 rounded-lg">
    <h1 class="text-2xl font-bold mb-6">Login to Nexus</h1>
    <form onsubmit={handleSubmit} class="space-y-4">
      <input
        type="text"
        bind:value={username}
        placeholder="Username"
        class="w-full p-3 bg-gray-700 rounded focus:outline-none focus:ring-2 focus:ring-indigo-500"
      />
      <input
        type="password"
        bind:value={password}
        placeholder="Password"
        class="w-full p-3 bg-gray-700 rounded focus:outline-none focus:ring-2 focus:ring-indigo-500"
      />
      <button
        type="submit"
        class="w-full p-3 bg-indigo-600 rounded font-semibold hover:bg-indigo-500 transition-colors"
      >
        Login
      </button>
    </form>
    <p class="mt-4 text-sm text-gray-400">
      Don't have an account?
      <a href="/register" class="text-indigo-400 hover:underline">Register</a>
    </p>
  </div>
</div>
