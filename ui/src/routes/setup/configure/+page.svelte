<script lang="ts">
	// ConfigureStep orchestrator (08_SETUP_WIZARD.md §4/5.3): requests a
	// recommendation for the selected models and renders an editable config card
	// per model. Per §5.3 the recommendation is merged with the user's overrides
	// AT RENDER — merged drafts are never stored; the store keeps only overrides.
	import { createRecommendationMutation } from '$lib/api/queries';
	import { queryClient } from '$lib/api/queryClient';
	import { useSetupStore } from '$lib/stores/setup.svelte';
	import ServerConfigCard from '$lib/components/setup/ServerConfigCard.svelte';
	import { mergeServerDraft } from '$lib/components/setup/serverDraft';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import type {
		BinaryScanResult,
		ConfigRecommendation,
		DiscoveredModel,
		ModelScanResult,
		RecommendationResult,
		ServerConfigDraft
	} from '$lib/api/types';

	const store = useSetupStore();
	const recommend = createRecommendationMutation(store.sessionId);

	const scanned = $derived(
		queryClient.getQueryData<ModelScanResult>(['setup', 'scan-models'])?.models ?? []
	);
	const selectedModels = $derived(scanned.filter((m) => store.selectedModelIds.has(m.id)));

	const binaryPath = $derived.by(() => {
		const scan = queryClient.getQueryData<BinaryScanResult>(['setup', 'scan-binary']);
		return scan?.found ? scan.path : '';
	});

	let hasRequested = $state(false);
	$effect(() => {
		if (hasRequested || selectedModels.length === 0) return;
		hasRequested = true;
		recommend.mutate({
			models: selectedModels.map((m) => ({ path: m.path, size_bytes: m.size_bytes }))
		});
	});

	function recommendationFor(model: DiscoveredModel): ConfigRecommendation | undefined {
		const result = recommend.data as RecommendationResult | undefined;
		return result?.servers.find((s) => s.model_path === model.path)?.recommended;
	}

	// Merge recommendation (base) with the user's stored overrides at render time.
	function valuesFor(model: DiscoveredModel, rec: ConfigRecommendation): ServerConfigDraft {
		return mergeServerDraft(model, rec, binaryPath, store.userOverrides.get(model.id) ?? {});
	}

	function onchange(model: DiscoveredModel, next: ServerConfigDraft) {
		store.userOverrides.set(model.id, next);
	}
</script>

<h1 tabindex="-1" class="text-2xl font-semibold text-[var(--color-text-primary)]">Configure</h1>
<p class="mt-2 text-[var(--color-text-secondary)]">
	Review and tune the recommended configuration for each model.
</p>

<div class="mt-6 space-y-4">
	{#if selectedModels.length === 0}
		<EmptyState
			title="No models selected"
			description="Go back to the Models step and choose at least one model."
			icon="📦"
		/>
	{:else if recommend.isPending}
		<Skeleton height="8rem" />
		<Skeleton height="8rem" />
	{:else if recommend.isError}
		<ErrorBanner
			message="Could not compute a recommendation."
			action={{ label: 'Retry', onClick: () => (hasRequested = false) }}
		/>
	{:else if recommend.data}
		{#each selectedModels as model (model.id)}
			{@const rec = recommendationFor(model)}
			{#if rec}
				<ServerConfigCard
					{model}
					recommendation={rec}
					values={valuesFor(model, rec)}
					onchange={(next) => onchange(model, next)}
				/>
			{/if}
		{/each}
	{/if}
</div>
