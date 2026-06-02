<script lang="ts">
	import { createConfigQuery } from '$lib/api/queries';
	import type { GlobalSettings } from '$lib/api/types';
	import GlobalSettingsEditor from '$lib/components/config/GlobalSettingsEditor.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';

	const q = createConfigQuery<GlobalSettings>('global');

	async function reload() {
		const r = await q.refetch();
		return { data: r.data!.data, etag: r.data!.etag };
	}
</script>

<svelte:head>
	<title>Global Settings | LlamaForge</title>
</svelte:head>

<div class="space-y-2">
	<h1 tabindex="-1" class="text-3xl font-bold text-[var(--color-text-primary)]">Global Settings</h1>
	<p class="text-sm text-[var(--color-text-secondary)]">
		Tune daemon-wide network, logging, queue, and orchestration settings.
	</p>
</div>

{#if q.isPending}
	<div
		class="space-y-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6"
	>
		<Skeleton height="3rem" />
		<Skeleton height="3rem" />
		<Skeleton height="3rem" />
	</div>
{:else if q.isError}
	<EmptyState
		title="Failed to load configuration"
		description="There was an error communicating with the backend API."
		icon="❌"
		action={{ label: 'Retry', onClick: () => q.refetch() }}
	/>
{:else}
	<GlobalSettingsEditor initial={q.data.data} etag={q.data.etag} {reload} />
{/if}
