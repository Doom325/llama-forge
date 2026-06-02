<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { createModelsQuery } from '$lib/api/queries';
	import { loadHighlighter } from '$lib/utils/markdown';
	import { toastStore } from '$lib/stores/toast.svelte';
	import { createChatSession } from '$lib/utils/chatSession.svelte';
	import {
		DEFAULT_TEMPERATURE,
		DEFAULT_MAX_TOKENS,
		DEFAULT_TOP_P,
		DEFAULT_TOP_K
	} from '$lib/utils/constants';
	import type { GenerationParams, ModelInfo } from '$lib/api/types';
	import ChatPane from '$lib/components/playground/ChatPane.svelte';
	import ParameterSidebar from '$lib/components/playground/ParameterSidebar.svelte';
	import ModelSelector from '$lib/components/playground/ModelSelector.svelte';
	import ComparisonPane from '$lib/components/playground/ComparisonPane.svelte';

	const modelsQuery = createModelsQuery();

	const defaults: GenerationParams = {
		temperature: DEFAULT_TEMPERATURE,
		max_tokens: DEFAULT_MAX_TOKENS,
		top_p: DEFAULT_TOP_P,
		top_k: DEFAULT_TOP_K
	};

	// URL-synced params (explorer pattern): read from page.url, fall back to defaults.
	const currentParams = $derived.by<GenerationParams>(() => {
		const sp = new URL(page.url).searchParams;
		const num = (key: string, fallback: number) => {
			const raw = sp.get(key);
			const parsed = raw === null ? NaN : Number(raw);
			return Number.isFinite(parsed) ? parsed : fallback;
		};
		return {
			temperature: num('temperature', defaults.temperature),
			max_tokens: num('max_tokens', defaults.max_tokens),
			top_p: num('top_p', defaults.top_p),
			top_k: num('top_k', defaults.top_k)
		};
	});

	const models = $derived<ModelInfo[]>(modelsQuery.data ?? []);

	const selectedModel = $derived.by<ModelInfo | null>(() => {
		const id = new URL(page.url).searchParams.get('model');
		if (id) {
			const match = models.find((m) => m.id === id);
			if (match) return match;
		}
		return models[0] ?? null;
	});

	function writeUrl(patch: Record<string, string>, replace: boolean) {
		const url = new URL(page.url);
		for (const [key, value] of Object.entries(patch)) {
			url.searchParams.set(key, value);
		}
		goto(url.toString(), { keepFocus: true, noScroll: true, replaceState: replace });
	}

	function updateParams(next: GenerationParams) {
		// replaceState: param sliders fire oninput continuously during a drag —
		// pushing history entries would make Back unusable. Tuning is not a
		// navigation milestone.
		writeUrl(
			{
				temperature: String(next.temperature),
				max_tokens: String(next.max_tokens),
				top_p: String(next.top_p),
				top_k: String(next.top_k)
			},
			true
		);
	}

	function updateModel(model: ModelInfo) {
		writeUrl({ model: model.id }, false);
	}

	function resetParams() {
		updateParams(defaults);
	}

	// Single source of truth for the primary conversation's streaming + state.
	const session = createChatSession();

	// Seed the URL with default params on first mount if absent, so deep links
	// and the sidebar always reflect concrete values (replaceState — no history).
	let seeded = false;
	$effect(() => {
		if (seeded) return;
		seeded = true;
		const sp = new URL(page.url).searchParams;
		if (!sp.has('temperature') || !sp.has('max_tokens') || !sp.has('top_p') || !sp.has('top_k')) {
			writeUrl(
				{
					temperature: String(defaults.temperature),
					max_tokens: String(defaults.max_tokens),
					top_p: String(defaults.top_p),
					top_k: String(defaults.top_k)
				},
				true
			);
		}
	});

	// Lazy-load the syntax highlighter once on mount.
	$effect(() => {
		loadHighlighter();
	});

	// --- Comparison mode -----------------------------------------------------

	let comparing = $state(false);
	let comparisonModelId = $state<string | null>(null);

	// The model rendered in the comparison pane: explicit selection, else the
	// first model that differs from the primary, else the primary itself.
	const comparisonModel = $derived.by<ModelInfo | null>(() => {
		if (comparisonModelId) {
			const match = models.find((m) => m.id === comparisonModelId);
			if (match) return match;
		}
		const primaryId = selectedModel?.id;
		return models.find((m) => m.id !== primaryId) ?? selectedModel ?? null;
	});

	// Latest prompt forwarded to the comparison pane so it generates the same
	// thing. ComparisonPane guards against resending the same value.
	let sharedPrompt = $state<string | null>(null);

	function toggleCompare() {
		comparing = !comparing;
	}

	function closeComparison() {
		comparing = false;
	}

	function handleComparisonModelChange(e: Event & { currentTarget: HTMLSelectElement }) {
		comparisonModelId = e.currentTarget.value;
	}

	// Narrow layout (<1280px) swaps the two-column grid for an ARIA tabs widget.
	let isNarrow = $state(false);
	$effect(() => {
		const mql = window.matchMedia('(max-width: 1279px)');
		isNarrow = mql.matches;
		const onChange = (e: MediaQueryListEvent) => {
			isNarrow = e.matches;
		};
		mql.addEventListener('change', onChange);
		return () => mql.removeEventListener('change', onChange);
	});

	// Roving-tabindex tabs widget for the narrow layout. Both panels stay mounted
	// (we only toggle `hidden`) so neither stream is interrupted on tab switch.
	type TabKey = 'primary' | 'comparison';
	let activeTab = $state<TabKey>('primary');

	function selectTab(tab: TabKey) {
		activeTab = tab;
	}

	function handleTabKeydown(e: KeyboardEvent) {
		if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
		e.preventDefault();
		const next: TabKey = activeTab === 'primary' ? 'comparison' : 'primary';
		activeTab = next;
		const id = next === 'primary' ? 'pg-tab-primary' : 'pg-tab-comparison';
		document.getElementById(id)?.focus();
	}

	async function handleSend(content: string) {
		if (!selectedModel) {
			toastStore.add('error', 'Select a model before sending a message.');
			return;
		}
		await session.send(content, selectedModel, currentParams);
		// Mirror the prompt to the comparison pane (if open) so both models
		// answer the same thing. Set after the primary send is dispatched.
		sharedPrompt = content;
	}
</script>

<svelte:head>
	<title>Playground | LlamaForge</title>
</svelte:head>

<div class="mx-auto max-w-[1400px] space-y-6">
	<div
		class="flex flex-col gap-4 border-b border-[var(--color-border)] pb-6 sm:flex-row sm:items-center sm:justify-between"
	>
		<div>
			<h1 tabindex="-1" class="text-3xl font-bold text-[var(--color-text-primary)]">Playground</h1>
			<p class="text-sm text-[var(--color-text-secondary)]">
				Stream chat completions and tune generation parameters interactively.
			</p>
		</div>
		<button
			type="button"
			class="inline-flex items-center gap-2 rounded-md border px-4 py-2 text-sm font-medium focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)] {comparing
				? 'border-[var(--color-focus-ring)] bg-[var(--color-focus-ring)]/10 text-[var(--color-focus-ring)]'
				: 'border-[var(--color-border)] bg-[var(--color-bg-secondary)] text-[var(--color-text-primary)] hover:bg-[var(--color-border)]'}"
			onclick={toggleCompare}
			aria-pressed={comparing}
			disabled={models.length === 0}
		>
			Compare
		</button>
	</div>

	<!-- Polite live region for queue/completion announcements. -->
	<span aria-live="polite" class="sr-only">{session.liveMessage}</span>

	<div class="grid grid-cols-1 gap-6 lg:grid-cols-[1fr_18rem]">
		<div class="order-2 lg:order-1">
			{#if comparing && comparisonModel}
				{#if isNarrow}
					<!-- Narrow: proper ARIA tabs widget. Panels stay mounted. -->
					<div role="tablist" aria-label="Playground panes" class="mb-3 flex gap-1">
						<button
							id="pg-tab-primary"
							type="button"
							role="tab"
							aria-selected={activeTab === 'primary'}
							aria-controls="pg-panel-primary"
							tabindex={activeTab === 'primary' ? 0 : -1}
							class="rounded-md px-4 py-2 text-sm font-medium focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)] {activeTab ===
							'primary'
								? 'bg-[var(--color-focus-ring)] text-white'
								: 'bg-[var(--color-bg-secondary)] text-[var(--color-text-primary)] hover:bg-[var(--color-border)]'}"
							onclick={() => selectTab('primary')}
							onkeydown={handleTabKeydown}
						>
							Primary
						</button>
						<button
							id="pg-tab-comparison"
							type="button"
							role="tab"
							aria-selected={activeTab === 'comparison'}
							aria-controls="pg-panel-comparison"
							tabindex={activeTab === 'comparison' ? 0 : -1}
							class="rounded-md px-4 py-2 text-sm font-medium focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)] {activeTab ===
							'comparison'
								? 'bg-[var(--color-focus-ring)] text-white'
								: 'bg-[var(--color-bg-secondary)] text-[var(--color-text-primary)] hover:bg-[var(--color-border)]'}"
							onclick={() => selectTab('comparison')}
							onkeydown={handleTabKeydown}
						>
							{comparisonModel.name}
						</button>
					</div>

					<div
						id="pg-panel-primary"
						role="tabpanel"
						aria-labelledby="pg-tab-primary"
						hidden={activeTab !== 'primary'}
						class="flex h-[70vh] min-h-[28rem] flex-col"
					>
						<ChatPane
							messages={session.messages}
							streamingState={session.streamingState}
							onSend={handleSend}
							onCancel={session.cancel}
							onClear={session.clear}
						/>
					</div>

					<div
						id="pg-panel-comparison"
						role="tabpanel"
						aria-labelledby="pg-tab-comparison"
						hidden={activeTab !== 'comparison'}
						class="flex h-[70vh] min-h-[28rem] flex-col"
					>
						<ComparisonPane
							model={comparisonModel}
							parameters={currentParams}
							{sharedPrompt}
							onClose={closeComparison}
						/>
					</div>
				{:else}
					<!-- Wide: primary + comparison side-by-side. -->
					<div class="grid grid-cols-1 gap-6 xl:grid-cols-2">
						<div class="flex h-[70vh] min-h-[28rem] flex-col">
							<ChatPane
								messages={session.messages}
								streamingState={session.streamingState}
								onSend={handleSend}
								onCancel={session.cancel}
								onClear={session.clear}
							/>
						</div>
						<div class="flex h-[70vh] min-h-[28rem] flex-col">
							<ComparisonPane
								model={comparisonModel}
								parameters={currentParams}
								{sharedPrompt}
								onClose={closeComparison}
							/>
						</div>
					</div>
				{/if}
			{:else}
				<div class="flex h-[70vh] min-h-[28rem] flex-col">
					<ChatPane
						messages={session.messages}
						streamingState={session.streamingState}
						onSend={handleSend}
						onCancel={session.cancel}
						onClear={session.clear}
					/>
				</div>
			{/if}
		</div>

		<div class="order-1 space-y-6 lg:order-2">
			<ModelSelector {models} selected={selectedModel} onSelect={updateModel} />

			{#if comparing}
				<div class="space-y-1">
					<label
						for="comparison-model-selector"
						class="block text-sm font-medium text-[var(--color-text-secondary)]"
					>
						Comparison model
					</label>
					<select
						id="comparison-model-selector"
						class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none"
						value={comparisonModel?.id ?? ''}
						onchange={handleComparisonModelChange}
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
			{/if}

			<ParameterSidebar
				params={currentParams}
				{defaults}
				onChange={updateParams}
				onReset={resetParams}
			/>
		</div>
	</div>
</div>
