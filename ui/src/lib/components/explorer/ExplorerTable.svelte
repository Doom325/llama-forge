<script lang="ts">
	import type { RequestRecord } from '$lib/api/types';
	import { formatDuration } from '$lib/utils/format';
	import {
		createTable,
		getCoreRowModel,
		getSortedRowModel,
		type ColumnDef,
		type SortingState,
		type Header,
		type CellContext,
		type Updater,
		type Table
	} from '@tanstack/table-core';
	import { createVirtualizer } from '@tanstack/svelte-virtual';
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';

	let {
		data,
		onRowClick
	}: {
		data: RequestRecord[];
		onRowClick: (req: RequestRecord) => void;
	} = $props();

	let sorting = $state<SortingState>([]);
	let rowSelection = $state({});

	const columns: ColumnDef<RequestRecord>[] = [
		{
			accessorKey: 'id',
			header: 'Request ID',
			cell: (info: CellContext<RequestRecord, unknown>) => info.getValue()
		},
		{
			accessorKey: 'timestamp',
			header: 'Timestamp',
			cell: (info: CellContext<RequestRecord, unknown>) =>
				new Date(info.getValue() as string).toLocaleString()
		},
		{
			accessorKey: 'model',
			header: 'Model',
			cell: (info: CellContext<RequestRecord, unknown>) => info.getValue()
		},
		{
			accessorKey: 'taskType',
			header: 'Task Type',
			cell: (info: CellContext<RequestRecord, unknown>) => info.getValue()
		},
		{
			accessorKey: 'duration',
			header: 'Duration',
			cell: (info: CellContext<RequestRecord, unknown>) => formatDuration(info.getValue() as number)
		},
		{
			accessorKey: 'status',
			header: 'Status',
			cell: (info: CellContext<RequestRecord, unknown>) => {
				const status = info.getValue() as string;
				return status; // Using the real badge in the raw cell render below
			}
		}
	];

	let tableOptions = $derived({
		data,
		columns,
		state: {
			sorting,
			rowSelection
		},
		onSortingChange: (updater: Updater<SortingState>) => {
			if (typeof updater === 'function') {
				sorting = updater(sorting);
			} else {
				sorting = updater;
			}
		},
		getCoreRowModel: getCoreRowModel(),
		getSortedRowModel: getSortedRowModel(),
		onStateChange: () => {},
		renderFallbackValue: null
	});

	let table = $derived.by(() => {
		const t = createTable(
			tableOptions as unknown as import('@tanstack/table-core').TableOptionsResolved<RequestRecord>
		) as unknown as Table<RequestRecord>;
		// table-core's getState() returns options.state verbatim — it does NOT merge
		// the feature-provided defaults. A partially-controlled state (sorting only)
		// therefore leaves slices like columnPinning undefined, and getHeaderGroups
		// crashes reading `columnPinning.left`. Merge the full initialState in, exactly
		// as the official TanStack table adapters do.
		t.setOptions((prev) => ({
			...prev,
			state: { ...t.initialState, ...prev.state }
		}));
		return t;
	});

	// Virtualizer setup
	let tableContainer: HTMLDivElement | undefined = $state();

	let virtualizerStore = $derived(
		tableContainer
			? createVirtualizer({
					count: table.getRowModel().rows.length,
					getScrollElement: () => tableContainer!,
					estimateSize: () => 50, // 50px row height
					overscan: 10
				})
			: undefined
	);

	let virtualRows = $derived(virtualizerStore ? $virtualizerStore?.getVirtualItems() || [] : []);
	let totalSize = $derived(virtualizerStore ? $virtualizerStore?.getTotalSize() || 0 : 0);

	// APG Grid Navigation logic
	let focusedLogicalIndex = $state(-1);

	// Preserve focus on DOM recycle
	$effect(() => {
		if (focusedLogicalIndex >= 0 && tableContainer) {
			const activeElement = document.activeElement as HTMLElement;
			// If focus is lost or shifted to the container because the row unmounted
			if (!activeElement || activeElement === tableContainer || activeElement === document.body) {
				const isVisible = virtualRows.some(
					(vr: { index: number }) => vr.index === focusedLogicalIndex
				);
				if (isVisible) {
					// Restore focus
					const rowEl = tableContainer.querySelector(
						`tr[data-index="${focusedLogicalIndex}"]`
					) as HTMLElement;
					if (rowEl) rowEl.focus();
				} else {
					// Announce
					announceMsg = `Row ${focusedLogicalIndex + 1} is no longer visible.`;
					tableContainer.focus();
				}
			}
		}
	});

	let announceMsg = $state('');

	function handleKeydown(e: KeyboardEvent, row: RequestRecord, index: number) {
		if (e.key === 'ArrowDown') {
			e.preventDefault();
			focusedLogicalIndex = Math.min(index + 1, data.length - 1);
			const nextRow = tableContainer?.querySelector(
				`tr[data-index="${focusedLogicalIndex}"]`
			) as HTMLElement;
			if (nextRow) nextRow.focus();
			else tableContainer?.focus(); // Fallback if virtualization means it's not rendered yet
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			focusedLogicalIndex = Math.max(index - 1, 0);
			const prevRow = tableContainer?.querySelector(
				`tr[data-index="${focusedLogicalIndex}"]`
			) as HTMLElement;
			if (prevRow) prevRow.focus();
			else tableContainer?.focus();
		} else if (e.key === 'Enter') {
			e.preventDefault();
			onRowClick(row);
		}
	}

	function handleRowFocus(index: number) {
		focusedLogicalIndex = index;
	}

	function handleSort(header: Header<RequestRecord, unknown>) {
		if (header.column.getCanSort()) {
			header.column.toggleSorting();
		}
	}

	function renderHeader(headerDef: unknown, context: unknown) {
		if (typeof headerDef === 'function') {
			return headerDef(context);
		}
		return headerDef as string;
	}

	function renderCell(cellDef: unknown, context: unknown) {
		if (typeof cellDef === 'function') {
			return cellDef(context);
		}
		return cellDef as string;
	}
</script>

<div class="sr-only" aria-live="polite">{announceMsg}</div>

<div
	bind:this={tableContainer}
	class="h-[600px] overflow-auto rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)]"
	tabindex="-1"
>
	<table
		class="relative w-full border-collapse text-left text-sm whitespace-nowrap"
		role="grid"
		aria-label="Requests Explorer"
	>
		<thead
			class="sticky top-0 z-10 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-sm"
		>
			{#each table.getHeaderGroups() as headerGroup (headerGroup.id)}
				<tr>
					{#each headerGroup.headers as header (header.id)}
						{@const isSorted = header.column.getIsSorted()}
						{@const canSort = header.column.getCanSort()}
						<th
							scope="col"
							class="px-6 py-3 font-semibold text-[var(--color-text-secondary)] {canSort
								? 'cursor-pointer select-none hover:bg-[var(--color-bg-primary)]'
								: ''}"
							aria-sort={isSorted === 'asc'
								? 'ascending'
								: isSorted === 'desc'
									? 'descending'
									: canSort
										? 'none'
										: undefined}
							onclick={() => handleSort(header)}
							onkeydown={(e) => (e.key === 'Enter' || e.key === ' ' ? handleSort(header) : null)}
							tabindex={canSort ? 0 : -1}
						>
							{#if !header.isPlaceholder}
								<div class="flex items-center gap-2">
									{renderHeader(header.column.columnDef.header, header.getContext())}
									{#if isSorted === 'asc'}
										<span aria-hidden="true">↑</span>
									{:else if isSorted === 'desc'}
										<span aria-hidden="true">↓</span>
									{/if}
								</div>
							{/if}
						</th>
					{/each}
				</tr>
			{/each}
		</thead>
		<tbody
			class="divide-y divide-[var(--color-border)]"
			style="height: {totalSize}px; position: relative;"
		>
			{#each virtualRows as virtualRow (virtualRow.index)}
				{@const row = table.getRowModel().rows[virtualRow.index]}
				{@const req = row!.original}
				<tr
					data-index={virtualRow.index}
					class="absolute w-full cursor-pointer transition-colors outline-none hover:bg-[var(--color-bg-secondary)]/50 focus:bg-[var(--color-bg-secondary)]"
					style="top: 0; left: 0; transform: translateY({virtualRow.start}px); height: {virtualRow.size}px;"
					tabindex={focusedLogicalIndex === virtualRow.index ? 0 : -1}
					onkeydown={(e) => handleKeydown(e, req, virtualRow.index)}
					onclick={() => onRowClick(req)}
					onfocus={() => handleRowFocus(virtualRow.index)}
				>
					{#each row!.getVisibleCells() as cell (cell.id)}
						<td class="px-6 py-3 {cell.column.id === 'id' ? 'font-mono text-xs' : ''}">
							{#if cell.column.id === 'status'}
								<StatusBadge
									status={req.status === 'success'
										? 'healthy'
										: req.status === 'error'
											? 'failed'
											: 'pending'}
									customLabel={req.status}
									size="sm"
								/>
							{:else}
								{renderCell(cell.column.columnDef.cell, cell.getContext())}
							{/if}
						</td>
					{/each}
				</tr>
			{/each}
		</tbody>
	</table>
</div>
