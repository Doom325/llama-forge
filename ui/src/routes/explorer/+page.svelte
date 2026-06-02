<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { createQuery } from '@tanstack/svelte-query';
	import { apiFetch } from '$lib/api/client';
	import ExplorerFilters, {
		type ExplorerFilterState
	} from '$lib/components/explorer/ExplorerFilters.svelte';
	import ExplorerTable from '$lib/components/explorer/ExplorerTable.svelte';
	import RequestDetailDrawer from '$lib/components/explorer/RequestDetailDrawer.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import type { RequestRecord } from '$lib/api/types';

	// URL Sync
	let currentFilters = $derived.by(() => {
		const url = new URL(page.url);
		return {
			model: url.searchParams.get('model') || '',
			taskType: url.searchParams.get('taskType') || '',
			timeRange: url.searchParams.get('timeRange') || '1h',
			search: url.searchParams.get('search') || ''
		} as ExplorerFilterState;
	});

	function updateFilters(newFilters: ExplorerFilterState) {
		const url = new URL(page.url);
		Object.entries(newFilters).forEach(([key, value]) => {
			if (value) {
				url.searchParams.set(key, value);
			} else {
				url.searchParams.delete(key);
			}
		});
		goto(url.toString(), { keepFocus: true, noScroll: true });
	}

	const qs = $derived(
		new URLSearchParams(currentFilters as unknown as Record<string, string>).toString()
	);

	const requestsQuery = createQuery(() => ({
		queryKey: ['requests', qs],
		queryFn: () => {
			return apiFetch<RequestRecord[]>(`/api/requests?${qs}`);
		},
		staleTime: 30_000,
		gcTime: 300_000 // 5 min
	}));

	let selectedRequest = $state<RequestRecord | null>(null);
	let drawerOpen = $state(false);

	function handleRowClick(req: RequestRecord) {
		selectedRequest = req;
		drawerOpen = true;

		// In a real app we'd map this back to logical index, but for simplicity
		// we just rely on ExplorerTable's internal focusedLogicalIndex if possible
	}

	function handleDrawerClose() {
		drawerOpen = false;
		// Return focus to table container or active row
		const table = document.querySelector('[role="grid"]') as HTMLElement;
		if (table) {
			const activeRow = table.querySelector('tr[tabindex="0"]') as HTMLElement;
			if (activeRow) activeRow.focus();
			else table.focus();
		}
	}

	function handleExport() {
		const data = requestsQuery.data;
		if (!data || data.length === 0) return;

		// Convert to CSV
		const headers = ['id', 'timestamp', 'model', 'taskType', 'duration', 'status'];
		const csvContent = [
			headers.join(','),
			...data.map((row: RequestRecord) =>
				headers.map((header) => (row as unknown as Record<string, unknown>)[header]).join(',')
			)
		].join('\n');

		const blob = new Blob([csvContent], { type: 'text/csv;charset=utf-8;' });
		const url = URL.createObjectURL(blob);
		const link = document.createElement('a');
		link.setAttribute('href', url);
		link.setAttribute('download', `requests_export_${new Date().toISOString()}.csv`);
		document.body.appendChild(link);
		link.click();
		document.body.removeChild(link);
		URL.revokeObjectURL(url);
	}
</script>

<svelte:head>
	<title>Request Explorer | LlamaForge</title>
</svelte:head>

<div class="mx-auto max-w-[1400px] space-y-6">
	<div
		class="flex flex-col gap-4 border-b border-[var(--color-border)] pb-6 sm:flex-row sm:items-center sm:justify-between"
	>
		<div>
			<h1 tabindex="-1" class="text-3xl font-bold text-[var(--color-text-primary)]">
				Request Explorer
			</h1>
			<p class="text-sm text-[var(--color-text-secondary)]">
				Search, filter, and inspect historical generation requests.
			</p>
		</div>
		<button
			type="button"
			class="inline-flex items-center gap-2 rounded-md border border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-border)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)] disabled:opacity-50"
			onclick={handleExport}
			disabled={!requestsQuery.data || requestsQuery.data.length === 0}
		>
			<svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					stroke-width="2"
					d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
				/>
			</svg>
			Export CSV
		</button>
	</div>

	<ExplorerFilters filters={currentFilters} onChange={updateFilters} />

	{#if requestsQuery.isPending}
		<div
			class="space-y-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6"
		>
			<Skeleton height="3rem" />
			<Skeleton height="3rem" />
			<Skeleton height="3rem" />
			<Skeleton height="3rem" />
		</div>
	{:else if requestsQuery.isError}
		<EmptyState
			title="Failed to load requests"
			description="There was an error communicating with the backend API."
			icon="❌"
			action={{ label: 'Retry', onClick: () => requestsQuery.refetch() }}
		/>
	{:else if requestsQuery.data.length === 0}
		<EmptyState
			title="No requests found"
			description="Try adjusting your filters or search query."
			icon="🔍"
		/>
	{:else}
		<ExplorerTable data={requestsQuery.data} onRowClick={handleRowClick} />
	{/if}

	<RequestDetailDrawer request={selectedRequest} open={drawerOpen} onClose={handleDrawerClose} />
</div>
