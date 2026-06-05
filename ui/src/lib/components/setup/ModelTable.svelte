<script lang="ts">
	// Presentational table of discovered models with multi-select. Pure —
	// selection state is owned by the orchestrator; emits toggles via callbacks.
	import type { DiscoveredModel } from '$lib/api/types';

	let {
		models,
		selectedIds,
		ontoggle,
		ontoggleAll
	}: {
		models: DiscoveredModel[];
		selectedIds: Set<string>;
		ontoggle: (id: string) => void;
		ontoggleAll: (select: boolean) => void;
	} = $props();

	const allSelected = $derived(models.length > 0 && models.every((m) => selectedIds.has(m.id)));

	function formatBytes(bytes: number): string {
		if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
		if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`;
		return `${bytes} B`;
	}

	function formatModified(value: string | undefined): string {
		if (!value) return '—';
		const d = new Date(value);
		return Number.isNaN(d.getTime()) ? value : d.toLocaleDateString();
	}
</script>

<table class="w-full text-left text-sm">
	<thead class="border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
		<tr>
			<th scope="col" class="p-2">
				<input
					type="checkbox"
					checked={allSelected}
					onchange={(e) => ontoggleAll(e.currentTarget.checked)}
					aria-label="Select all models"
				/>
			</th>
			<th scope="col" class="p-2">Model</th>
			<th scope="col" class="p-2">Quantization</th>
			<th scope="col" class="p-2">Architecture</th>
			<th scope="col" class="p-2">Size</th>
			<th scope="col" class="p-2">Modified</th>
		</tr>
	</thead>
	<tbody>
		{#each models as model (model.id)}
			<tr class="border-b border-[var(--color-border)]">
				<td class="p-2">
					<input
						type="checkbox"
						checked={selectedIds.has(model.id)}
						onchange={() => ontoggle(model.id)}
						aria-label={`Select ${model.filename}`}
					/>
				</td>
				<td class="p-2 font-mono text-xs text-[var(--color-text-primary)]">{model.filename}</td>
				<td class="p-2">{model.quantization_hint}</td>
				<td class="p-2">{model.architecture_hint}</td>
				<td class="p-2">{formatBytes(model.size_bytes)}</td>
				<td class="p-2 text-[var(--color-text-secondary)]">{formatModified(model.modified)}</td>
			</tr>
		{/each}
	</tbody>
</table>
