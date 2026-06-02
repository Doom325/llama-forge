<script lang="ts">
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
	import DynamicHeading from '$lib/components/shared/DynamicHeading.svelte';
	import type { ServerState, ServerMetricsSummary, DetailServerAction } from '$lib/api/types';
	import { formatTokensPerSecond, formatDuration } from '$lib/utils/format';
	import { base } from '$app/paths';

	let {
		server,
		metrics,
		isFocused = false,
		availableActions,
		onAction,
		onHover
	}: {
		server: ServerState;
		metrics?: ServerMetricsSummary;
		isFocused?: boolean;
		availableActions: DetailServerAction[];
		onAction: (id: string, action: DetailServerAction) => void;
		onHover?: () => void;
	} = $props();
</script>

<a
	href="{base}/servers/{server.id}"
	role="gridcell"
	aria-label="Server {server.name}"
	tabindex={isFocused ? 0 : -1}
	class="block flex flex-col gap-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-5 no-underline transition-colors hover:border-[var(--color-text-secondary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
	style="min-height: var(--card-min-height, 180px);"
	onmouseenter={onHover}
>
	<div class="flex items-start justify-between">
		<div>
			<DynamicHeading level={3} class="text-lg font-bold text-[var(--color-text-primary)]"
				>{server.name}</DynamicHeading
			>
			<div class="mt-1 font-mono text-xs text-[var(--color-text-secondary)]">ID: {server.id}</div>
		</div>
		<StatusBadge status={server.status} />
	</div>

	<div class="grid grid-cols-2 gap-3 text-sm">
		<div class="flex flex-col rounded-lg bg-[var(--color-bg-secondary)] p-3">
			<span class="mb-1 text-[var(--color-text-secondary)]">Throughput</span>
			<span class="font-medium">{metrics ? formatTokensPerSecond(metrics.throughput) : '--'}</span>
		</div>
		<div class="flex flex-col rounded-lg bg-[var(--color-bg-secondary)] p-3">
			<span class="mb-1 text-[var(--color-text-secondary)]">Uptime</span>
			<span class="font-medium">{formatDuration(server.uptime)}</span>
		</div>
		<div class="col-span-2 flex flex-col rounded-lg bg-[var(--color-bg-secondary)] p-3">
			<span class="mb-1 text-[var(--color-text-secondary)]">Slots (Idle/Proc)</span>
			<span class="font-medium">
				{#if metrics}
					{metrics.slots.idle} / {metrics.slots.processing} (Total: {metrics.slots.total})
				{:else}
					--
				{/if}
			</span>
		</div>
	</div>

	<div class="mt-auto flex justify-end gap-2 pt-2">
		{#if availableActions.includes('start')}
			<button
				type="button"
				tabindex={isFocused ? 0 : -1}
				class="rounded-md bg-[var(--color-success-fill)] px-3 py-1.5 text-sm font-medium text-white hover:opacity-90 focus:outline-none focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]"
				onclick={(e) => {
					e.preventDefault();
					e.stopPropagation();
					onAction(server.id, 'start');
				}}
			>
				Start
			</button>
		{/if}
		{#if availableActions.includes('restart')}
			<button
				type="button"
				tabindex={isFocused ? 0 : -1}
				class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-1.5 text-sm font-medium hover:bg-[var(--color-bg-secondary)] focus:outline-none focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]"
				onclick={(e) => {
					e.preventDefault();
					e.stopPropagation();
					onAction(server.id, 'restart');
				}}
			>
				Restart
			</button>
		{/if}
		{#if availableActions.includes('stop')}
			<button
				type="button"
				tabindex={isFocused ? 0 : -1}
				class="rounded-md border border-[var(--color-status-failed)] px-3 py-1.5 text-sm font-medium text-[var(--color-status-failed)] hover:bg-[var(--color-status-failed)]/10 focus:outline-none focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]"
				onclick={(e) => {
					e.preventDefault();
					e.stopPropagation();
					onAction(server.id, 'stop');
				}}
			>
				Stop
			</button>
		{/if}
	</div>
</a>
