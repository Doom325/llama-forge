<script lang="ts">
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
	import DynamicHeading from '$lib/components/shared/DynamicHeading.svelte';
	import type { ModelInfo } from '$lib/api/types';

	let { model }: { model: ModelInfo } = $props();
</script>

<div
	class="flex flex-col gap-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-5"
>
	<div class="flex items-start justify-between gap-3">
		<div class="min-w-0">
			<DynamicHeading level={3} class="truncate text-lg font-bold text-[var(--color-text-primary)]"
				>{model.name}</DynamicHeading
			>
			<div class="mt-1 truncate font-mono text-xs text-[var(--color-text-secondary)]">
				ID: {model.id}
			</div>
		</div>
		{#if model.loaded}
			<StatusBadge status="healthy" customLabel="Loaded" />
		{:else}
			<StatusBadge status="pending" customLabel="Not loaded" />
		{/if}
	</div>

	<div>
		<div
			class="mb-1 text-xs font-medium tracking-wide text-[var(--color-text-secondary)] uppercase"
		>
			Assigned servers
		</div>
		{#if model.assignedServers.length === 0}
			<p class="text-sm text-[var(--color-text-secondary)]">None</p>
		{:else}
			<ul class="flex flex-wrap gap-1.5">
				{#each model.assignedServers as server (server)}
					<li
						class="rounded-full border border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-2.5 py-0.5 font-mono text-xs text-[var(--color-text-primary)]"
					>
						{server}
					</li>
				{/each}
			</ul>
		{/if}
	</div>

	<div>
		<div
			class="mb-1 text-xs font-medium tracking-wide text-[var(--color-text-secondary)] uppercase"
		>
			Capabilities
		</div>
		{#if model.capabilities.length === 0}
			<p class="text-sm text-[var(--color-text-secondary)]">None</p>
		{:else}
			<ul class="flex flex-wrap gap-1.5">
				{#each model.capabilities as capability (capability)}
					<li
						class="rounded-full border border-[var(--color-focus-ring)]/30 bg-[var(--color-focus-ring)]/10 px-2.5 py-0.5 text-xs font-medium text-[var(--color-focus-ring)]"
					>
						{capability}
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>
