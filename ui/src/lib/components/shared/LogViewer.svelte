<script lang="ts">
	// Shared scrollable log pane: fixed-height <pre> that auto-tails new lines
	// (unless the user has scrolled up), announces appends via aria-live, and is
	// keyboard-scrollable. Reusable for build logs and future server logs.

	let {
		lines,
		label = 'Log output'
	}: {
		lines: string[];
		label?: string;
	} = $props();

	let pre = $state<HTMLPreElement | null>(null);
	let pinnedToBottom = $state(true);

	// Auto-tail: when new lines arrive and the view is pinned to the bottom,
	// scroll to follow. Reading `lines.length` registers the reactive dependency.
	$effect(() => {
		void lines.length;
		if (pinnedToBottom && pre) {
			pre.scrollTop = pre.scrollHeight;
		}
	});

	function onScroll() {
		if (!pre) return;
		pinnedToBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 8;
	}
</script>

<!-- A focusable overflow:auto region is natively arrow-key scrollable when
     focused, so no explicit key handler is needed. `onscroll` only tracks
     whether the user has scrolled away from the tail (to pause auto-tail).
     tabindex=0 is intentional: a long scrollable log must be reachable by
     keyboard to scroll it (WCAG 2.1.1), the documented exception to the rule. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<pre
	bind:this={pre}
	onscroll={onScroll}
	tabindex="0"
	role="log"
	aria-live="polite"
	aria-label={label}
	class="h-[400px] overflow-auto rounded border border-[var(--color-border)] bg-[var(--color-bg-secondary)] p-3 font-mono text-xs whitespace-pre-wrap text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none">{#each lines as line, index (index)}{line}
	{/each}</pre>
