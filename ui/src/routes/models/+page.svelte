<script lang="ts">
	import { createModelsQuery } from '$lib/api/queries';
	import ModelCard from '$lib/components/models/ModelCard.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';

	const q = createModelsQuery();
</script>

<svelte:head>
	<title>Models | LlamaForge</title>
</svelte:head>

<div class="mx-auto max-w-[1400px] space-y-6">
	<div class="border-b border-[var(--color-border)] pb-6">
		<h1 tabindex="-1" class="text-3xl font-bold text-[var(--color-text-primary)]">Models</h1>
		<p class="text-sm text-[var(--color-text-secondary)]">
			Inspect available models, their load status, assigned servers, and capabilities.
		</p>
	</div>

	{#if q.isPending}
		<div class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
			{#each Array(6), i (i)}
				<div
					class="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-5"
				>
					<Skeleton height="1.5rem" width="60%" />
					<Skeleton height="1rem" width="40%" class="mt-3" />
					<Skeleton height="2rem" class="mt-4" />
				</div>
			{/each}
		</div>
	{:else if q.isError}
		<EmptyState
			title="Failed to load models"
			description="There was an error communicating with the backend API."
			icon="❌"
			action={{ label: 'Retry', onClick: () => q.refetch() }}
		/>
	{:else if q.data.length === 0}
		<EmptyState
			title="No models"
			description="No models are currently registered with the backend."
			icon="📦"
		/>
	{:else}
		<div class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
			{#each q.data as model (model.id)}
				<ModelCard {model} />
			{/each}
		</div>
	{/if}
</div>
