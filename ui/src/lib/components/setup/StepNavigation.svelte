<script lang="ts">
	// Presentational footer: Back / Skip / Next. Pure — emits intent via callback
	// props; the orchestrator owns navigation and advancement rules.

	let {
		canGoBack,
		canAdvance,
		canSkip = false,
		nextLabel = 'Next',
		isLoading = false,
		onback,
		onnext,
		onskip
	}: {
		canGoBack: boolean;
		canAdvance: boolean;
		canSkip?: boolean;
		nextLabel?: string;
		isLoading?: boolean;
		onback?: () => void;
		onnext?: () => void;
		onskip?: () => void;
	} = $props();
</script>

<div class="flex items-center justify-between border-t border-[var(--color-border)] pt-4">
	<button
		type="button"
		onclick={() => onback?.()}
		disabled={!canGoBack || isLoading}
		class="rounded px-4 py-2 text-sm font-medium text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-focus-ring)] focus:outline-none disabled:cursor-not-allowed disabled:opacity-40"
	>
		Back
	</button>

	<div class="flex items-center gap-3">
		{#if canSkip}
			<button
				type="button"
				onclick={() => onskip?.()}
				disabled={isLoading}
				class="rounded px-4 py-2 text-sm font-medium text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-focus-ring)] focus:outline-none disabled:opacity-40"
			>
				Skip
			</button>
		{/if}
		<button
			type="button"
			onclick={() => onnext?.()}
			disabled={!canAdvance || isLoading}
			class="rounded bg-[var(--color-focus-ring)] px-5 py-2 text-sm font-medium text-white focus:ring-2 focus:ring-[var(--color-focus-ring)] focus:ring-offset-2 focus:outline-none disabled:cursor-not-allowed disabled:opacity-40"
		>
			{isLoading ? 'Working…' : nextLabel}
		</button>
	</div>
</div>
