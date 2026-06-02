<script lang="ts">
	export interface ExplorerFilterState {
		model: string;
		taskType: string;
		timeRange: string;
		search: string;
	}

	let {
		filters = { model: '', taskType: '', timeRange: '1h', search: '' },
		onChange
	}: {
		filters?: ExplorerFilterState;
		onChange: (filters: ExplorerFilterState) => void;
	} = $props();

	function handleChange(e: Event) {
		const target = e.target as HTMLInputElement | HTMLSelectElement;
		onChange({ ...filters, [target.name]: target.value });
	}

	function handleSearch(e: Event) {
		const target = e.target as HTMLInputElement;
		// A bit of debounce could be added here in a real app, but direct onChange for now
		onChange({ ...filters, search: target.value });
	}
</script>

<div
	class="flex flex-col flex-wrap gap-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-4 sm:flex-row sm:items-end"
>
	<div class="min-w-[200px] flex-1 space-y-1">
		<label for="filter-search" class="block text-sm font-medium text-[var(--color-text-secondary)]"
			>Search</label
		>
		<input
			type="text"
			id="filter-search"
			name="search"
			value={filters.search}
			oninput={handleSearch}
			placeholder="Search requests..."
			class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] placeholder:text-[var(--color-text-secondary)]/50 focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		/>
	</div>

	<div class="min-w-[150px] space-y-1">
		<label for="filter-model" class="block text-sm font-medium text-[var(--color-text-secondary)]"
			>Model</label
		>
		<select
			id="filter-model"
			name="model"
			value={filters.model}
			onchange={handleChange}
			class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		>
			<option value="">All Models</option>
			<option value="llama3">llama3</option>
			<option value="mistral">mistral</option>
			<option value="qwen">qwen</option>
		</select>
	</div>

	<div class="min-w-[150px] space-y-1">
		<label
			for="filter-taskType"
			class="block text-sm font-medium text-[var(--color-text-secondary)]">Task Type</label
		>
		<select
			id="filter-taskType"
			name="taskType"
			value={filters.taskType}
			onchange={handleChange}
			class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		>
			<option value="">All Tasks</option>
			<option value="chat">Chat</option>
			<option value="completion">Completion</option>
			<option value="embedding">Embedding</option>
		</select>
	</div>

	<div class="min-w-[150px] space-y-1">
		<label
			for="filter-timeRange"
			class="block text-sm font-medium text-[var(--color-text-secondary)]">Time Range</label
		>
		<select
			id="filter-timeRange"
			name="timeRange"
			value={filters.timeRange}
			onchange={handleChange}
			class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		>
			<option value="1h">Last 1 Hour</option>
			<option value="24h">Last 24 Hours</option>
			<option value="7d">Last 7 Days</option>
			<option value="all">All Time</option>
		</select>
	</div>
</div>
