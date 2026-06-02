<script lang="ts">
	import { liveDataStore } from '$lib/api/queryClient';
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
</script>

{#if !liveDataStore.connected}
	<div aria-live="polite">
		<StatusBadge status="reconnecting" size="sm" />
	</div>
{:else}
	<span aria-live="polite" class="sr-only">Connected</span>
{/if}
