<script lang="ts">
	import type { Message, StreamingState } from '$lib/api/types';
	import MessageBubble from './MessageBubble.svelte';
	import StreamingDisplay from './StreamingDisplay.svelte';

	let {
		messages,
		streamingState,
		onSend,
		onCancel,
		onClear
	}: {
		messages: Message[];
		streamingState: StreamingState;
		onSend: (content: string) => void;
		onCancel: () => void;
		onClear: () => void;
	} = $props();

	let draft = $state('');

	const isBusy = $derived(
		streamingState.status === 'queued' || streamingState.status === 'streaming'
	);

	function submit() {
		const content = draft.trim();
		if (!content || isBusy) return;
		draft = '';
		onSend(content);
	}

	function handleKeydown(e: KeyboardEvent) {
		// Enter sends; Shift+Enter inserts a newline.
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			submit();
		}
	}

	function retry() {
		// Re-send the last user message verbatim.
		const lastUser = [...messages].reverse().find((m) => m.role === 'user');
		if (lastUser) onSend(lastUser.content);
	}
</script>

<div class="flex h-full flex-col gap-4">
	<div
		class="flex-1 space-y-4 overflow-y-auto rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-secondary)] p-4"
		role="log"
		aria-label="Conversation"
	>
		{#if messages.length === 0 && streamingState.status === 'idle'}
			<p class="text-sm text-[var(--color-text-secondary)]">
				Send a message to start a conversation.
			</p>
		{/if}

		{#each messages as message, i (i)}
			<MessageBubble {message} />
		{/each}

		{#if streamingState.status === 'streaming'}
			<StreamingDisplay tokens={streamingState.tokens} isStreaming={true} />
		{/if}

		{#if streamingState.status === 'queued'}
			<p class="text-sm text-[var(--color-text-secondary)]" role="status">
				Queued — position {streamingState.position}{#if streamingState.eta !== null}, ETA ~{streamingState.eta}s{/if}
			</p>
		{/if}

		{#if streamingState.status === 'error'}
			<div
				class="space-y-2 rounded-lg border border-[var(--color-status-failed)] bg-[var(--color-bg-primary)] p-4"
			>
				{#if streamingState.partialResponse}
					<StreamingDisplay tokens={streamingState.partialResponse} isStreaming={false} />
				{/if}
				<p class="text-sm text-[var(--color-status-failed)]" role="alert">
					{streamingState.error.message}
				</p>
				{#if streamingState.error.retryable}
					<button
						type="button"
						class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-3 py-1.5 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-border)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)]"
						onclick={retry}
					>
						Retry
					</button>
				{/if}
			</div>
		{/if}
	</div>

	<div class="space-y-2">
		<label for="chat-input" class="sr-only">Message</label>
		<textarea
			id="chat-input"
			class="w-full resize-y rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none disabled:opacity-50"
			rows="3"
			placeholder="Type a message… (Enter to send, Shift+Enter for newline)"
			bind:value={draft}
			onkeydown={handleKeydown}
			disabled={isBusy}
		></textarea>

		<div class="flex items-center gap-2">
			{#if streamingState.status === 'streaming' || streamingState.status === 'queued'}
				<button
					type="button"
					class="rounded-md border border-[var(--color-status-failed)] bg-[var(--color-bg-secondary)] px-4 py-2 text-sm font-medium text-[var(--color-status-failed)] hover:opacity-80 focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)]"
					onclick={onCancel}
				>
					Cancel
				</button>
			{:else}
				<button
					type="button"
					class="rounded-md bg-[var(--color-accent-fill)] px-4 py-2 text-sm font-medium text-white hover:opacity-90 focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)] disabled:opacity-50"
					onclick={submit}
					disabled={draft.trim().length === 0}
				>
					Send
				</button>
			{/if}
			<button
				type="button"
				class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-border)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)] disabled:opacity-50"
				onclick={onClear}
				disabled={isBusy || messages.length === 0}
			>
				Clear
			</button>
		</div>
	</div>
</div>
