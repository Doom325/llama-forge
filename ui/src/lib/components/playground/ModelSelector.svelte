<script lang="ts">
	import type { ModelInfo } from '$lib/api/types';

	let {
		models,
		selected,
		onSelect
	}: {
		models: ModelInfo[];
		selected: ModelInfo | null;
		onSelect: (m: ModelInfo) => void;
	} = $props();

	function handleChange(e: Event & { currentTarget: HTMLSelectElement }) {
		const next = models.find((m) => m.id === e.currentTarget.value);
		if (next) onSelect(next);
	}
</script>

<div class="space-y-1">
	<label for="model-selector" class="block text-sm font-medium text-[var(--color-text-secondary)]">
		Model
	</label>
	<select
		id="model-selector"
		class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
		value={selected?.id ?? ''}
		onchange={handleChange}
		disabled={models.length === 0}
	>
		{#if models.length === 0}
			<option value="">No models available</option>
		{:else}
			{#each models as model (model.id)}
				<option value={model.id}>{model.name}</option>
			{/each}
		{/if}
	</select>
</div>
