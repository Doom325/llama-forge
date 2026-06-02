<script lang="ts">
	import type { Message } from '$lib/api/types';
	import { renderMarkdown } from '$lib/utils/markdown';
	import ResponseMetrics from './ResponseMetrics.svelte';

	let { message }: { message: Message } = $props();

	const isUser = $derived(message.role === 'user');
</script>

<div class="flex flex-col gap-1" class:items-end={isUser}>
	<div
		class="max-w-[90%] rounded-lg px-4 py-3 text-sm"
		class:user-bubble={isUser}
		class:assistant-bubble={!isUser}
	>
		<span class="mb-1 block text-xs font-semibold tracking-wide uppercase opacity-70">
			{isUser ? 'You' : 'Assistant'}
		</span>
		{#if isUser}
			<p class="break-words whitespace-pre-wrap">{message.content}</p>
		{:else}
			<!-- renderMarkdown() is the single sanitized {@html} source, enforced by
			     the local/no-raw-html-without-sanitize rule. The built-in
			     svelte/no-at-html-tags rule is redundant given that guard. -->
			<div class="prose prose-sm max-w-none break-words">
				<!-- eslint-disable-next-line svelte/no-at-html-tags -->
				{@html renderMarkdown(message.content)}
			</div>
		{/if}
	</div>
	{#if message.metrics}
		<div class="px-1">
			<ResponseMetrics metrics={message.metrics} />
		</div>
	{/if}
</div>

<style>
	.user-bubble {
		background-color: var(--color-bg-secondary);
		color: var(--color-text-primary);
		border: 1px solid var(--color-border);
	}
	.assistant-bubble {
		background-color: var(--color-bg-primary);
		color: var(--color-text-primary);
		border: 1px solid var(--color-border);
	}
</style>
