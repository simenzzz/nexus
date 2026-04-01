<script lang="ts">
  let username = $state('');
  let displayName = $state('');
  let password = $state('');
  let confirmPassword = $state('');

  function handleSubmit(e: Event) {
    e.preventDefault();
    // TODO: Validate passwords match, call API register endpoint
  }
</script>

<div class="flex items-center justify-center min-h-screen">
  <div class="w-full max-w-md p-8 bg-gray-800 rounded-lg">
    <h1 class="text-2xl font-bold mb-6">Create your Nexus account</h1>
    <form onsubmit={handleSubmit} class="space-y-4">
      <input
        type="text"
        bind:value={username}
        placeholder="Username"
        class="w-full p-3 bg-gray-700 rounded focus:outline-none focus:ring-2 focus:ring-indigo-500"
      />
      <input
        type="text"
        bind:value={displayName}
        placeholder="Display Name"
        class="w-full p-3 bg-gray-700 rounded focus:outline-none focus:ring-2 focus:ring-indigo-500"
      />
      <input
        type="password"
        bind:value={password}
        placeholder="Password"
        class="w-full p-3 bg-gray-700 rounded focus:outline-none focus:ring-2 focus:ring-indigo-500"
      />
      <input
        type="password"
        bind:value={confirmPassword}
        placeholder="Confirm Password"
        class="w-full p-3 bg-gray-700 rounded focus:outline-none focus:ring-2 focus:ring-indigo-500"
      />
      <button
        type="submit"
        class="w-full p-3 bg-indigo-600 rounded font-semibold hover:bg-indigo-500 transition-colors"
      >
        Register
      </button>
    </form>
    <p class="mt-4 text-sm text-gray-400">
      Already have an account?
      <a href="/login" class="text-indigo-400 hover:underline">Login</a>
    </p>
  </div>
</div>
