<script lang="ts">
	let { tokens, isStreaming }: { tokens: string; isStreaming: boolean } = $props();

	// Track the streaming→done transition so the polite region only announces
	// completion once (and never floods AT while tokens stream in).
	let announce = $state('');
	let wasStreaming = $state(false);
	$effect(() => {
		if (isStreaming) {
			wasStreaming = true;
			announce = '';
		} else if (wasStreaming) {
			wasStreaming = false;
			announce = 'Generation complete.';
		}
	});
</script>

<div
	class="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-4 py-3 text-sm text-[var(--color-text-primary)]"
>
	<span class="mb-1 block text-xs font-semibold tracking-wide uppercase opacity-70">Assistant</span>
	<!-- Plain text while streaming to avoid re-parsing markdown on every token.
	     aria-live="off": token-by-token updates would flood a screen reader. -->
	<p class="break-words whitespace-pre-wrap" aria-live="off">
		{tokens}{#if isStreaming}<span class="streaming-cursor" aria-hidden="true">▋</span>{/if}
	</p>
</div>

<span aria-live="polite" class="sr-only">{announce}</span>

<style>
	.streaming-cursor {
		display: inline-block;
		animation: blink 1s step-start infinite;
		color: var(--color-text-secondary);
	}
	@keyframes blink {
		50% {
			opacity: 0;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.streaming-cursor {
			animation: none;
		}
	}
</style>
