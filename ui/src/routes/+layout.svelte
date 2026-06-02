<script lang="ts">
	import '../app.css';
	import { initializeTheme } from '$lib/stores/theme.svelte';
	import { browser } from '$app/environment';
	import { QueryClientProvider } from '@tanstack/svelte-query';
	import { queryClient, liveDataStore } from '$lib/api/queryClient';
	import Toast from '$lib/components/shared/Toast.svelte';
	import AppShell from '$lib/components/layout/AppShell.svelte';
	import Sidebar from '$lib/components/layout/Sidebar.svelte';
	import Header from '$lib/components/layout/Header.svelte';
	import { afterNavigate } from '$app/navigation';

	let { children } = $props();

	$effect(() => {
		if (browser) {
			const disposeTheme = initializeTheme();
			const disposeLive = liveDataStore.subscribe();
			return () => {
				if (disposeTheme) disposeTheme();
				disposeLive();
			};
		}
	});

	afterNavigate(() => {
		const mainH1 = document.querySelector('main h1') as HTMLElement;
		if (mainH1) {
			mainH1.focus();
		}
	});

	// Remove the app.html pre-hydration skeleton here rather than via an inline
	// <script>, so the app runs under a strict CSP (no 'unsafe-inline' for
	// scripts; SvelteKit hashes its own inline bootstrap — see svelte.config.js).
	$effect(() => {
		document.getElementById('pre-hydration-skeleton')?.remove();
	});
</script>

<QueryClientProvider client={queryClient}>
	<AppShell>
		<Sidebar />
		<Header />
		<main id="main-content" class="skeleton-main overflow-y-auto" tabindex="-1">
			{@render children()}
		</main>
	</AppShell>
</QueryClientProvider>

<Toast />
