<script lang="ts">
	let {
		width = '100%',
		height = '1rem',
		variant = 'text',
		class: className = ''
	}: {
		width?: string;
		height?: string;
		variant?: 'text' | 'rect' | 'circle';
		class?: string;
	} = $props();

	let variantClass = $derived(
		variant === 'circle' ? 'rounded-full' : variant === 'text' ? 'rounded-md' : 'rounded-lg'
	);
</script>

<div
	aria-busy="true"
	aria-hidden="true"
	class="animate-pulse bg-[var(--color-border)] {variantClass} {className}"
	style="width: {width}; height: {height};"
></div>
