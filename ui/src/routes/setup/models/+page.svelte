<script lang="ts">
	// ModelStep orchestrator (08_SETUP_WIZARD.md §2.1/4): scans directories for
	// GGUF models and lets the user select which to serve. Selection lives in the
	// wizard store (client-owned); the scan result is server state via TanStack.
	import { createModelScanMutation } from '$lib/api/queries';
	import { useSetupStore } from '$lib/stores/setup.svelte';
	import ModelTable from '$lib/components/setup/ModelTable.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';

	const store = useSetupStore();
	const scan = createModelScanMutation(store.sessionId);

	// Common model locations seeded on entry. The backend rejects an *empty*
	// directory list (NoDirectories → 400) but silently skips ones that don't
	// exist, so a non-empty default scans cleanly (200, possibly empty) instead
	// of erroring before the user has added anything. No `~` entries — the backend
	// does not expand the home shorthand, so they would never match.
	const DEFAULT_SCAN_DIRS = ['./models'];

	let directories = $state<string[]>([...DEFAULT_SCAN_DIRS]);
	let dirInput = $state('');
	let hasScanned = $state(false);

	// Scan standard locations on entry; the user can add directories and re-scan.
	$effect(() => {
		if (!hasScanned) {
			hasScanned = true;
			scan.mutate({ source: 'local', directories });
		}
	});

	function rescan() {
		scan.mutate({ source: 'local', directories });
	}

	function addDirectory() {
		const dir = dirInput.trim();
		if (!dir) return;
		directories = [...directories, dir];
		dirInput = '';
		rescan();
	}

	function toggle(id: string) {
		if (store.selectedModelIds.has(id)) store.selectedModelIds.delete(id);
		else store.selectedModelIds.add(id);
	}

	function toggleAll(select: boolean) {
		for (const model of scan.data?.models ?? []) {
			if (select) store.selectedModelIds.add(model.id);
			else store.selectedModelIds.delete(model.id);
		}
	}

	const fieldClass =
		'flex-1 rounded border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';
</script>

<h1 tabindex="-1" class="text-2xl font-semibold text-[var(--color-text-primary)]">Models</h1>
<p class="mt-2 text-[var(--color-text-secondary)]">
	Select the models you want to serve. Add directories to scan for more.
</p>

<div class="mt-6 space-y-4">
	<div class="flex gap-2">
		<input
			bind:value={dirInput}
			class={fieldClass}
			placeholder="/path/to/models"
			aria-label="Directory to scan"
		/>
		<button
			type="button"
			onclick={addDirectory}
			class="rounded border border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none"
		>
			Add &amp; scan
		</button>
	</div>

	{#if scan.isPending}
		<Skeleton height="3rem" />
		<Skeleton height="3rem" />
	{:else if scan.isError}
		<ErrorBanner
			message="Model scan failed."
			action={{ label: 'Retry', onClick: () => rescan() }}
		/>
	{:else if scan.data}
		{#if scan.data.truncated}
			<ErrorBanner
				variant="warning"
				message={`Results were truncated (${scan.data.truncation_reason}). Narrow the search directories for a complete list.`}
			/>
		{/if}
		{#if scan.data.models.length === 0}
			<EmptyState
				title="No models found"
				description="Add a directory above that contains .gguf model files."
				icon="📦"
			/>
		{:else}
			<ModelTable
				models={scan.data.models}
				selectedIds={store.selectedModelIds}
				ontoggle={toggle}
				ontoggleAll={toggleAll}
			/>
			<p class="text-sm text-[var(--color-text-secondary)]">
				{store.selectedModelIds.size} selected
			</p>
		{/if}
	{/if}
</div>
