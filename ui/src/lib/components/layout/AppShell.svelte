<script lang="ts">
	import { preferences } from '$lib/stores/preferences.svelte';
	import { onMount } from 'svelte';

	let { children } = $props();

	onMount(() => {
		const handleKeydown = (e: KeyboardEvent) => {
			if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b') {
				e.preventDefault();
				preferences.toggleSidebar();
			}
		};
		window.addEventListener('keydown', handleKeydown);
		return () => window.removeEventListener('keydown', handleKeydown);
	});
</script>

<a
	href="#main-content"
	class="sr-only focus:not-sr-only focus:absolute focus:top-4 focus:left-4 focus:z-50 focus:rounded-md focus:bg-[var(--color-bg-primary)] focus:px-4 focus:py-2 focus:font-bold focus:text-[var(--color-focus-ring)] focus:outline focus:outline-2 focus:outline-offset-2"
>
	Skip to main content
</a>

<div class="app-shell grid h-screen" class:collapsed={preferences.sidebarCollapsed}>
	{@render children()}
</div>

<style>
	:global(#app) {
		display: contents; /* Ensure the injected content doesn't break the parent grid */
	}

	.app-shell {
		display: contents;
	}

	:global(.app-shell > aside) {
		grid-column: 1;
		grid-row: 1 / 3;
	}

	:global(.app-shell > header) {
		grid-column: 2;
		grid-row: 1;
	}

	:global(.app-shell > main) {
		grid-column: 2;
		grid-row: 2;
	}

	@media (max-width: 768px) {
		:global(#app) {
			grid-template-columns: var(--sidebar-collapsed-width, 64px) 1fr !important;
		}
	}

	@media (max-width: 640px) {
		:global(#app) {
			grid-template-columns: 1fr !important;
		}

		:global(.app-shell > header),
		:global(.app-shell > main) {
			grid-column: 1;
		}
	}
</style>
