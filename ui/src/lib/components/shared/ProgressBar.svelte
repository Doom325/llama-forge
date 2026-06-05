<script lang="ts">
	// Shared progress bar. Determinate when `value` is a number (0–100),
	// indeterminate when null/undefined. Animation respects prefers-reduced-motion.

	let {
		value = null,
		label
	}: {
		value?: number | null;
		label?: string;
	} = $props();

	const determinate = $derived(value !== null && value !== undefined);
	const clamped = $derived(determinate ? Math.max(0, Math.min(100, value as number)) : 0);
</script>

<div class="space-y-1">
	{#if label}
		<div class="flex justify-between text-xs text-[var(--color-text-secondary)]">
			<span>{label}</span>
			{#if determinate}<span>{clamped}%</span>{/if}
		</div>
	{/if}
	<div
		role="progressbar"
		aria-label={label ?? 'Progress'}
		aria-valuemin={determinate ? 0 : undefined}
		aria-valuemax={determinate ? 100 : undefined}
		aria-valuenow={determinate ? clamped : undefined}
		class="h-2 w-full overflow-hidden rounded-full bg-[var(--color-border)]"
	>
		{#if determinate}
			<div
				class="h-full rounded-full bg-[var(--color-accent-fill)] transition-[width] duration-300"
				style:width="{clamped}%"
			></div>
		{:else}
			<div class="h-full w-full bg-[var(--color-accent-fill)] motion-safe:animate-pulse"></div>
		{/if}
	</div>
</div>
