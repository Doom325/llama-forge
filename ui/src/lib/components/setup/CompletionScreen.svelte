<script lang="ts">
	// Presentational completion state (§13.7). The orchestrator owns the
	// status-poll loop and feeds the resulting state in.
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';

	let {
		state,
		onretry,
		ongotodashboard
	}: {
		state: 'polling' | 'success' | 'timeout';
		onretry?: () => void;
		ongotodashboard?: () => void;
	} = $props();

	const primaryButton =
		'rounded bg-[var(--color-accent-fill)] px-5 py-2 text-sm font-medium text-white focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:ring-offset-2 focus:outline-none';
	const secondaryButton =
		'rounded border border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';
</script>

{#if state === 'polling'}
	<p class="text-sm text-[var(--color-text-secondary)]" role="status" aria-live="polite">
		Waiting for the orchestrator to start serving…
	</p>
{:else if state === 'success'}
	<div class="space-y-3">
		<div class="flex items-center gap-2">
			<StatusBadge status="healthy" size="sm" customLabel="Running" />
			<span class="text-sm font-medium text-[var(--color-text-primary)]">LlamaForge is running</span
			>
		</div>
		<p class="text-sm text-[var(--color-text-secondary)]">Setup is complete.</p>
		<button type="button" onclick={() => ongotodashboard?.()} class={primaryButton}>
			Go to Dashboard
		</button>
	</div>
{:else if state === 'timeout'}
	<div class="space-y-3">
		<ErrorBanner
			variant="warning"
			message="Setup was saved, but the server hasn't finished transitioning to running mode yet."
		/>
		<div class="flex flex-wrap gap-3">
			<button type="button" onclick={() => onretry?.()} class={primaryButton}>Retry</button>
			<button type="button" onclick={() => ongotodashboard?.()} class={secondaryButton}>
				Go to Dashboard anyway
			</button>
		</div>
	</div>
{/if}
