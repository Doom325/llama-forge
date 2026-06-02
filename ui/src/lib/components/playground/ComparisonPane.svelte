<script lang="ts">
	import type { GenerationParams, ModelInfo } from '$lib/api/types';
	import { createChatSession } from '$lib/utils/chatSession.svelte';
	import ChatPane from './ChatPane.svelte';

	let {
		model,
		parameters,
		sharedPrompt,
		onClose
	}: {
		model: ModelInfo;
		parameters: GenerationParams;
		sharedPrompt: string | null;
		onClose: () => void;
	} = $props();

	// Independent session: its own message list, StreamingState, and
	// AbortController so this stream is isolated from the primary pane.
	const session = createChatSession();

	// Polite announcement surfaced on mount (and if the model changes).
	const announcement = $derived(`Comparison pane opened for ${model.name}.`);

	// Track the last prompt we forwarded so unrelated re-renders (e.g. a param
	// tweak) don't resend the same shared prompt. A new send only fires when
	// sharedPrompt transitions to a non-null value we have not yet sent.
	let lastSent: string | null = null;
	$effect(() => {
		const prompt = sharedPrompt;
		if (prompt !== null && prompt !== lastSent) {
			lastSent = prompt;
			void session.send(prompt, model, parameters);
		}
	});

	function handleClose() {
		// Aborting cancels only THIS pane's in-flight stream.
		session.cancel();
		onClose();
	}

	function handleSend(content: string) {
		void session.send(content, model, parameters);
	}
</script>

<section
	aria-label={`Response from ${model.name} (comparison)`}
	class="flex h-full flex-col gap-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-4"
>
	<div class="flex items-center justify-between gap-2 border-b border-[var(--color-border)] pb-3">
		<h3 class="truncate text-lg font-bold text-[var(--color-text-primary)]">{model.name}</h3>
		<button
			type="button"
			class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-3 py-1.5 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-border)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)]"
			onclick={handleClose}
		>
			Close
		</button>
	</div>

	<span aria-live="polite" class="sr-only">{announcement}</span>

	<div class="min-h-0 flex-1">
		<ChatPane
			messages={session.messages}
			streamingState={session.streamingState}
			onSend={handleSend}
			onCancel={session.cancel}
			onClear={session.clear}
		/>
	</div>
</section>
