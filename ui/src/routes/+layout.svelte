<script lang="ts">
	import '../app.css';
	import { initializeTheme } from '$lib/stores/theme.svelte';
	import { browser } from '$app/environment';

	let { children } = $props();

	$effect(() => {
		if (browser) {
			const dispose = initializeTheme();
			return () => {
				if (dispose) dispose();
			};
		}
	});
</script>

{@render children()}
