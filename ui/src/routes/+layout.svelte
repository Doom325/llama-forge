<script lang="ts">
	import '../app.css';
	import { initializeTheme } from '$lib/stores/theme.svelte';
	import { browser } from '$app/environment';
	import { QueryClientProvider } from '@tanstack/svelte-query';
	import { queryClient } from '$lib/api/queryClient';
	import Toast from '$lib/components/shared/Toast.svelte';

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

<QueryClientProvider client={queryClient}>
	{@render children()}
</QueryClientProvider>

<Toast />
