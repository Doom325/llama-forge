<script lang="ts">
	// ReviewStep orchestrator (08_SETUP_WIZARD.md §13.5/13.7): preview the
	// generated TOML, save it, validate-launch the servers, then poll /api/status
	// until the orchestrator reports running. Owns all I/O for this step.
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import {
		createPreviewConfigMutation,
		createSetupSaveConfigMutation,
		createValidateLaunchMutation,
		fetchSetupStatus
	} from '$lib/api/queries';
	import { queryClient } from '$lib/api/queryClient';
	import { useSetupStore } from '$lib/stores/setup.svelte';
	import { mergeServerDraft } from '$lib/components/setup/serverDraft';
	import { renderMarkdown } from '$lib/utils/markdown';
	import SavePathSelector from '$lib/components/setup/SavePathSelector.svelte';
	import CompletionScreen from '$lib/components/setup/CompletionScreen.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import type {
		BinaryScanResult,
		ModelScanResult,
		RecommendationResult,
		ServerConfigDraft
	} from '$lib/api/types';

	const DEFAULT_PATH = '~/.config/llamaforge/llamaforge.toml';

	const store = useSetupStore();
	const preview = createPreviewConfigMutation(store.sessionId, store.reconfigure);
	const save = createSetupSaveConfigMutation(store.sessionId, store.reconfigure);
	const validate = createValidateLaunchMutation(store.sessionId, store.reconfigure);

	// Reconfigure overwrites the existing config (the backend merges); default the
	// overwrite toggle on so the save isn't blocked by the 409 conflict guard.
	let force = $state(store.reconfigure);
	let phase = $state<'review' | 'completion'>('review');
	let completionState = $state<'polling' | 'success' | 'timeout'>('polling');
	let actionError = $state<string | null>(null);
	let backupPath = $state<string | null>(null);
	let hasPreviewed = $state(false);

	const effectivePath = $derived(store.savePath.trim() || DEFAULT_PATH);

	const selectedModels = $derived(
		(queryClient.getQueryData<ModelScanResult>(['setup', 'scan-models'])?.models ?? []).filter(
			(m) => store.selectedModelIds.has(m.id)
		)
	);
	const binaryPath = $derived.by(() => {
		const scan = queryClient.getQueryData<BinaryScanResult>(['setup', 'scan-binary']);
		return scan?.found ? scan.path : '';
	});

	const servers = $derived.by<ServerConfigDraft[]>(() => {
		const rec = queryClient.getQueryData<RecommendationResult>(['setup', 'recommend']);
		if (!rec) return [];
		const drafts: ServerConfigDraft[] = [];
		for (const model of selectedModels) {
			const recommended = rec.servers.find((s) => s.model_path === model.path)?.recommended;
			if (recommended) {
				drafts.push(
					mergeServerDraft(model, recommended, binaryPath, store.userOverrides.get(model.id) ?? {})
				);
			}
		}
		return drafts;
	});

	// Preview the assembled config once the servers are known.
	$effect(() => {
		if (hasPreviewed || servers.length === 0) return;
		hasPreviewed = true;
		preview.mutate({ path: effectivePath, force, servers });
	});

	// Set on unmount so an in-flight poll loop stops issuing /api/status GETs
	// (and stops writing completionState) after the user navigates away.
	let cancelled = false;
	$effect(() => () => {
		cancelled = true;
	});

	async function pollStatus() {
		completionState = 'polling';
		for (let i = 0; i < 5; i++) {
			await new Promise((resolve) => setTimeout(resolve, 2000));
			if (cancelled) return;
			try {
				const status = await fetchSetupStatus();
				if (cancelled) return;
				if (!status.setup_required) {
					completionState = 'success';
					return;
				}
			} catch {
				// transient — keep polling
			}
		}
		if (!cancelled) completionState = 'timeout';
	}

	async function saveAndLaunch() {
		actionError = null;
		try {
			const saved = await save.mutateAsync({ path: effectivePath, force, servers });
			backupPath = saved.backup_path ?? null;
			const result = await validate.mutateAsync(servers);
			if (!result.success) {
				actionError = result.stderr ? `${result.message}\n${result.stderr}` : result.message;
				return;
			}
			phase = 'completion';
			pollStatus();
		} catch (e) {
			actionError = e instanceof Error ? e.message : 'Save failed.';
		}
	}

	const busy = $derived(save.isPending || validate.isPending);
</script>

<h1 tabindex="-1" class="text-2xl font-semibold text-[var(--color-text-primary)]">Review</h1>

<div class="mt-6 space-y-6">
	{#if phase === 'completion'}
		<CompletionScreen
			state={completionState}
			onretry={pollStatus}
			ongotodashboard={() => goto(`${base}/`)}
		/>
		{#if backupPath}
			<p class="text-xs text-[var(--color-text-secondary)]">
				Previous config backed up to <code>{backupPath}</code>.
			</p>
		{/if}
	{:else}
		<section class="space-y-2">
			<h2 class="text-sm font-medium text-[var(--color-text-primary)]">Generated configuration</h2>
			{#if preview.isPending}
				<Skeleton height="10rem" />
			{:else if preview.data}
				<div class="overflow-auto rounded border border-[var(--color-border)] text-xs">
					<!-- renderMarkdown() is the only sanitized {@html} source (enforced by
					     no-raw-html-without-sanitize). -->
					<!-- eslint-disable-next-line svelte/no-at-html-tags -->
					{@html renderMarkdown('```toml\n' + preview.data.toml + '\n```')}
				</div>
			{/if}
		</section>

		<SavePathSelector
			path={store.savePath}
			{force}
			placeholder={DEFAULT_PATH}
			onpathchange={(value) => (store.savePath = value)}
			onforcechange={(value) => (force = value)}
		/>

		{#if actionError}
			<ErrorBanner message={actionError} />
		{/if}

		<button
			type="button"
			onclick={saveAndLaunch}
			disabled={busy || servers.length === 0}
			class="rounded bg-[var(--color-accent-fill)] px-6 py-3 text-sm font-medium text-white focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:ring-offset-2 focus:outline-none disabled:cursor-not-allowed disabled:opacity-40"
		>
			{busy ? 'Saving…' : 'Save & Launch'}
		</button>
	{/if}
</div>
