<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		level,
		children,
		class: className = ''
	}: {
		level: 1 | 2 | 3 | 4 | 5 | 6;
		children: Snippet;
		class?: string;
	} = $props();
</script>

{#if level === 1}
	<h1 class={className}>{@render children()}</h1>
{:else if level === 2}
	<h2 class={className}>{@render children()}</h2>
{:else if level === 3}
	<h3 class={className}>{@render children()}</h3>
{:else if level === 4}
	<h4 class={className}>{@render children()}</h4>
{:else if level === 5}
	<h5 class={className}>{@render children()}</h5>
{:else if level === 6}
	<h6 class={className}>{@render children()}</h6>
{/if}
