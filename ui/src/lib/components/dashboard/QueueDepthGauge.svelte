<script lang="ts">
	let { depth, capacity }: { depth: number; capacity: number } = $props();

	let percentage = $derived(capacity > 0 ? Math.min(Math.round((depth / capacity) * 100), 100) : 0);

	let colorClass = $derived(
		percentage < 50
			? 'bg-[var(--color-status-healthy)]'
			: percentage < 80
				? 'bg-[var(--color-status-degraded)]'
				: 'bg-[var(--color-status-failed)]'
	);
</script>

<div class="flex w-full flex-col gap-1">
	<div class="flex justify-between text-sm font-medium text-[var(--color-text-secondary)]">
		<span>Queue Depth</span>
		<span aria-hidden="true">{depth} / {capacity}</span>
	</div>
	<div
		role="meter"
		aria-valuenow={depth}
		aria-valuemin={0}
		aria-valuemax={capacity}
		aria-label="Queue depth: {depth} of {capacity}"
		class="h-2 w-full overflow-hidden rounded-full bg-[var(--color-bg-secondary)]"
	>
		<div class="h-full {colorClass} transition-all duration-300" style="width: {percentage}%"></div>
	</div>
</div>
