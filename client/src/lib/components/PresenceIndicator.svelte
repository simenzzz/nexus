<script lang="ts">
  import type { UserStatus } from '$stores/presence';

  interface Props {
    status: UserStatus;
  }

  let { status }: Props = $props();

  const statusColors: Record<UserStatus, string> = {
    online: 'bg-green-500',
    idle: 'bg-yellow-500',
    dnd: 'bg-red-500',
    offline: 'bg-gray-500',
  };
</script>

<span class="inline-block w-3 h-3 rounded-full border-2 border-gray-800 {statusColors[status]}"></span>
