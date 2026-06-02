<script lang="ts">
	import { createQueueQuery, createCancelRequestMutation } from '$lib/api/queries';
	import QueueFilters from '$lib/components/queue/QueueFilters.svelte';
	import QueueTable from '$lib/components/queue/QueueTable.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import { toastStore } from '$lib/stores/toast.svelte';
	import { SvelteSet } from 'svelte/reactivity';

	const queueQuery = createQueueQuery();
	const cancelMutation = createCancelRequestMutation();

	let filters = $state({ model: '', requestor: '', status: '' });

	// Optimistic UI state
	let pendingCancels = new SvelteSet<string>();

	// Filter items on the client since the queue query pulls the entire queue
	let filteredQueue = $derived.by(() => {
		let items = queueQuery.data || [];

		if (filters.model) {
			const modelLower = filters.model.toLowerCase();
			items = items.filter((i) => i.model.toLowerCase().includes(modelLower));
		}

		if (filters.requestor) {
			const reqLower = filters.requestor.toLowerCase();
			items = items.filter((i) => i.requestor.toLowerCase().includes(reqLower));
		}

		if (filters.status) {
			items = items.filter((i) => i.status === filters.status);
		}

		return items;
	});

	// Cleanup pending cancels if they are naturally removed from the queue
	$effect(() => {
		const currentIds = new Set((queueQuery.data || []).map((i) => i.id));

		if (pendingCancels.size > 0) {
			for (const id of Array.from(pendingCancels)) {
				if (!currentIds.has(id)) {
					pendingCancels.delete(id);
				}
			}
		}
	});

	function handleCancel(id: string) {
		// Optimistic UI
		pendingCancels.add(id);

		cancelMutation.mutate(id, {
			onError: (err) => {
				// Revert optimistic state
				pendingCancels.delete(id);

				toastStore.add('error', `Failed to cancel request ${id}: ${err.message}`);
			}
		});
	}
</script>

<svelte:head>
	<title>Queue Inspector | LlamaForge</title>
</svelte:head>

<div class="mx-auto max-w-[1400px] space-y-6">
	<div class="flex flex-col gap-2 border-b border-[var(--color-border)] pb-6">
		<h1 tabindex="-1" class="text-3xl font-bold text-[var(--color-text-primary)]">
			Queue Inspector
		</h1>
		<p class="text-sm text-[var(--color-text-secondary)]">
			Monitor and manage queued generation requests.
		</p>
	</div>

	<QueueFilters {filters} onChange={(f) => (filters = f)} />

	{#if queueQuery.isPending}
		<div
			class="space-y-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6"
		>
			<Skeleton height="3rem" />
			<Skeleton height="3rem" />
			<Skeleton height="3rem" />
			<Skeleton height="3rem" />
		</div>
	{:else if queueQuery.isError}
		<EmptyState
			title="Failed to load queue"
			description="There was an error communicating with the backend API."
			icon="❌"
			action={{ label: 'Retry', onClick: () => queueQuery.refetch() }}
		/>
	{:else if filteredQueue.length === 0}
		<EmptyState
			title="Queue is empty"
			description={queueQuery.data?.length === 0
				? 'There are currently no requests waiting or processing.'
				: 'No requests match the current filters.'}
			icon="✓"
		/>
	{:else}
		<QueueTable items={filteredQueue} {pendingCancels} onCancel={handleCancel} />
	{/if}
</div>
