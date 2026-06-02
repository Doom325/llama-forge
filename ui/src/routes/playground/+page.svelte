<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { createModelsQuery } from '$lib/api/queries';
	import { streamChatCompletion, isAbortError } from '$lib/api/chat';
	import { ApiError } from '$lib/api/client';
	import { loadHighlighter } from '$lib/utils/markdown';
	import { toastStore } from '$lib/stores/toast.svelte';
	import {
		DEFAULT_TEMPERATURE,
		DEFAULT_MAX_TOKENS,
		DEFAULT_TOP_P,
		DEFAULT_TOP_K,
		TIMEOUTS
	} from '$lib/utils/constants';
	import type {
		ChatCompletionRequest,
		GenerationParams,
		Message,
		ModelInfo,
		ResponseMetrics,
		StreamingState
	} from '$lib/api/types';
	import ChatPane from '$lib/components/playground/ChatPane.svelte';
	import ParameterSidebar from '$lib/components/playground/ParameterSidebar.svelte';
	import ModelSelector from '$lib/components/playground/ModelSelector.svelte';

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

	let messages = $state<Message[]>([]);
	let streamingState = $state<StreamingState>({ status: 'idle' });
	let controller = $state<AbortController | null>(null);
	let liveMessage = $state('');

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

	// Rough token estimate (~4 chars/token) — the backend is authoritative, but
	// usage stats are not always returned mid-stream, so we approximate.
	function estimateTokens(text: string): number {
		return Math.max(1, Math.ceil(text.length / 4));
	}

	async function handleSend(content: string) {
		const model = selectedModel;
		if (!model) {
			toastStore.add('error', 'Select a model before sending a message.');
			return;
		}

		messages = [...messages, { role: 'user', content, timestamp: Date.now() }];

		// Go straight to streaming. The `queued` state + its announcement are wired
		// for when the backend reports a real queue position; we don't fake one.
		streamingState = { status: 'streaming', tokens: '', ttft: 0 };

		controller = new AbortController();
		const params = currentParams;
		const req: ChatCompletionRequest = {
			model: model.id,
			messages: messages.map((m) => ({ role: m.role, content: m.content })),
			temperature: params.temperature,
			max_tokens: params.max_tokens,
			top_p: params.top_p,
			top_k: params.top_k,
			stream: true
		};

		const start = Date.now();
		let ttft = 0;
		let tokens = '';

		try {
			await streamChatCompletion(
				req,
				AbortSignal.any([controller.signal, AbortSignal.timeout(TIMEOUTS.PLAYGROUND_ABORT)]),
				(delta) => {
					if (ttft === 0) ttft = Date.now() - start;
					tokens += delta;
					streamingState = { status: 'streaming', tokens, ttft };
				}
			);

			const elapsedMs = Date.now() - start;
			const totalTokens = estimateTokens(tokens);
			const tokensPerSecond = elapsedMs > 0 ? totalTokens / (elapsedMs / 1000) : 0;
			const metrics: ResponseMetrics = { ttft, tokensPerSecond, totalTokens };

			messages = [
				...messages,
				{ role: 'assistant', content: tokens, timestamp: Date.now(), metrics }
			];
			streamingState = { status: 'complete', response: tokens, metrics };
			liveMessage = 'Generation complete.';
			streamingState = { status: 'idle' };
		} catch (err) {
			if (isAbortError(err)) {
				// Graceful stop. AbortSignal.any propagates the originating signal's
				// reason, so a 120s timeout surfaces as TimeoutError (not AbortError);
				// controller.signal.aborted distinguishes user-cancel from timeout.
				const userCancelled = controller?.signal.aborted ?? false;
				if (tokens.length > 0) {
					messages = [...messages, { role: 'assistant', content: tokens, timestamp: Date.now() }];
				}
				streamingState = { status: 'idle' };
				liveMessage = userCancelled ? 'Generation stopped.' : 'Generation timed out.';
			} else {
				const message =
					err instanceof ApiError || err instanceof Error ? err.message : 'Generation failed.';
				streamingState = {
					status: 'error',
					error: { message, retryable: true },
					partialResponse: tokens.length > 0 ? tokens : null
				};
				liveMessage = 'Generation failed.';
				toastStore.add('error', message);
			}
		} finally {
			controller = null;
		}
	}

	function handleCancel() {
		controller?.abort();
	}

	function handleClear() {
		messages = [];
		streamingState = { status: 'idle' };
		liveMessage = '';
	}
</script>

<svelte:head>
	<title>Playground | LlamaForge</title>
</svelte:head>

<div class="mx-auto max-w-[1400px] space-y-6">
	<div class="border-b border-[var(--color-border)] pb-6">
		<h1 tabindex="-1" class="text-3xl font-bold text-[var(--color-text-primary)]">Playground</h1>
		<p class="text-sm text-[var(--color-text-secondary)]">
			Stream chat completions and tune generation parameters interactively.
		</p>
	</div>

	<!-- Polite live region for queue/completion announcements. -->
	<span aria-live="polite" class="sr-only">{liveMessage}</span>

	<div class="grid grid-cols-1 gap-6 lg:grid-cols-[1fr_18rem]">
		<div class="order-2 flex h-[70vh] min-h-[28rem] flex-col lg:order-1">
			<ChatPane
				{messages}
				{streamingState}
				onSend={handleSend}
				onCancel={handleCancel}
				onClear={handleClear}
			/>
		</div>

		<div class="order-1 space-y-6 lg:order-2">
			<ModelSelector {models} selected={selectedModel} onSelect={updateModel} />
			<ParameterSidebar
				params={currentParams}
				{defaults}
				onChange={updateParams}
				onReset={resetParams}
			/>
		</div>
	</div>
</div>
