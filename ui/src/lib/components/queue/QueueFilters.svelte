<script lang="ts">
	let {
		filters = { model: '', requestor: '', status: '' },
		onChange
	}: {
		filters?: { model: string; requestor: string; status: string };
		onChange: (filters: { model: string; requestor: string; status: string }) => void;
	} = $props();

	function handleChange(e: Event) {
		const target = e.target as HTMLInputElement | HTMLSelectElement;
		onChange({ ...filters, [target.name]: target.value });
	}
</script>

<div
	class="flex flex-col gap-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-4 sm:flex-row sm:items-end"
>
	<div class="flex-1 space-y-1">
		<label for="filter-model" class="block text-sm font-medium text-[var(--color-text-secondary)]"
			>Model</label
		>
		<input
			type="text"
			id="filter-model"
			name="model"
			value={filters.model}
			oninput={handleChange}
			placeholder="Filter by model..."
			class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] placeholder:text-[var(--color-text-secondary)]/50 focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		/>
	</div>

	<div class="flex-1 space-y-1">
		<label
			for="filter-requestor"
			class="block text-sm font-medium text-[var(--color-text-secondary)]">Requestor</label
		>
		<input
			type="text"
			id="filter-requestor"
			name="requestor"
			value={filters.requestor}
			oninput={handleChange}
			placeholder="Filter by requestor..."
			class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] placeholder:text-[var(--color-text-secondary)]/50 focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		/>
	</div>

	<div class="flex-1 space-y-1">
		<label for="filter-status" class="block text-sm font-medium text-[var(--color-text-secondary)]"
			>Status</label
		>
		<select
			id="filter-status"
			name="status"
			value={filters.status}
			onchange={handleChange}
			class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		>
			<option value="">All</option>
			<option value="queued">Queued</option>
			<option value="processing">Processing</option>
		</select>
	</div>
</div>
