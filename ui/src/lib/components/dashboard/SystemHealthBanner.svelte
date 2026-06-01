<script lang="ts">
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';

	let {
		healthStatus,
		serverCounts
	}: {
		healthStatus: 'healthy' | 'degraded' | 'failed';
		serverCounts: { degraded: number; failed: number; total: number };
	} = $props();

	let message = $derived(
		healthStatus === 'failed'
			? `${serverCounts.failed} of ${serverCounts.total} servers are in a failed state. System capacity is severely reduced.`
			: healthStatus === 'degraded'
				? `${serverCounts.degraded} of ${serverCounts.total} servers are operating in a degraded state.`
				: ''
	);
</script>

{#if healthStatus !== 'healthy'}
	<ErrorBanner variant={healthStatus === 'failed' ? 'error' : 'warning'} {message} />
{/if}
