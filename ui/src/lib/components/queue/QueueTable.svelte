<script lang="ts">
	import type { QueueItem } from '$lib/api/types';
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
	import { formatDuration } from '$lib/utils/format';

	import type { SvelteSet } from 'svelte/reactivity';

	let {
		items,
		pendingCancels = new Set<string>() as unknown as SvelteSet<string>,
		onCancel
	}: {
		items: QueueItem[];
		pendingCancels?: SvelteSet<string>;
		onCancel: (id: string) => void;
	} = $props();
</script>

<div
	class="overflow-x-auto rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)]"
>
	<table class="w-full text-left text-sm whitespace-nowrap">
		<thead class="border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
			<tr>
				<th
					scope="col"
					class="px-6 py-3 font-semibold text-[var(--color-text-secondary)]"
					aria-sort="ascending">Position</th
				>
				<th scope="col" class="px-6 py-3 font-semibold text-[var(--color-text-secondary)]"
					>Request ID</th
				>
				<th scope="col" class="px-6 py-3 font-semibold text-[var(--color-text-secondary)]">Model</th
				>
				<th scope="col" class="px-6 py-3 font-semibold text-[var(--color-text-secondary)]"
					>Requestor</th
				>
				<th scope="col" class="px-6 py-3 font-semibold text-[var(--color-text-secondary)]"
					>Status</th
				>
				<th scope="col" class="px-6 py-3 font-semibold text-[var(--color-text-secondary)]"
					>Wait Time</th
				>
				<th
					scope="col"
					class="px-6 py-3 text-right font-semibold text-[var(--color-text-secondary)]">Actions</th
				>
			</tr>
		</thead>
		<tbody class="divide-y divide-[var(--color-border)]">
			{#each items as item (item.id)}
				{@const isCancelling = pendingCancels.has(item.id)}
				<tr
					class="transition-colors hover:bg-[var(--color-bg-secondary)]/50"
					class:opacity-50={isCancelling}
				>
					<td class="px-6 py-4 font-medium">{item.position}</td>
					<td class="px-6 py-4 font-mono text-xs">{item.id}</td>
					<td class="px-6 py-4">{item.model}</td>
					<td class="px-6 py-4 text-[var(--color-text-secondary)]">{item.requestor}</td>
					<td class="px-6 py-4">
						{#if isCancelling}
							<StatusBadge status="reconnecting" customLabel="Cancelling..." size="sm" />
						{:else}
							<StatusBadge
								status={item.status === 'processing' ? 'healthy' : 'pending'}
								customLabel={item.status === 'processing' ? 'Processing' : 'Queued'}
								size="sm"
							/>
						{/if}
					</td>
					<td class="px-6 py-4">{formatDuration(item.waitTime)}</td>
					<td class="px-6 py-4 text-right">
						<button
							type="button"
							class="rounded-md border border-[var(--color-status-failed)] px-3 py-1.5 text-xs font-medium text-[var(--color-status-failed)] transition-colors hover:bg-[var(--color-status-failed)]/10 focus:outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)] disabled:cursor-not-allowed disabled:opacity-50"
							disabled={isCancelling}
							onclick={() => onCancel(item.id)}
							aria-label="Cancel request {item.id}"
						>
							{isCancelling ? 'Cancelling...' : 'Cancel'}
						</button>
					</td>
				</tr>
			{/each}
		</tbody>
	</table>
</div>
