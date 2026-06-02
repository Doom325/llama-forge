<script lang="ts">
	import ThemeToggle from './ThemeToggle.svelte';
	import ConnectionStatus from './ConnectionStatus.svelte';
	import KeyboardShortcuts from './KeyboardShortcuts.svelte';
	import { preferences } from '$lib/stores/preferences.svelte';
	import { dashboardStore } from '$lib/stores/dashboard.svelte';
	import { page } from '$app/state';

	let breadcrumb = $derived(
		page.url.pathname === '/'
			? 'Dashboard'
			: page.url.pathname
					.substring(1)
					.split('/')
					.map((s) => s.charAt(0).toUpperCase() + s.slice(1))
					.join(' / ')
	);
</script>

<header class="skeleton-header flex items-center justify-between px-6 transition-all duration-200">
	<div class="flex items-center gap-4">
		<!-- Mobile menu toggle button -->
		<button
			type="button"
			class="rounded-md p-2 hover:bg-[var(--color-bg-secondary)] md:hidden"
			aria-label={preferences.sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
			aria-expanded={!preferences.sidebarCollapsed}
			aria-controls="mobile-sidebar"
			onclick={() => preferences.toggleSidebar()}
		>
			<span aria-hidden="true">☰</span>
		</button>

		<div class="text-lg font-semibold" aria-hidden="true">{breadcrumb}</div>
	</div>

	<div class="flex items-center gap-4">
		{#if page.url.pathname === '/' && dashboardStore.announceSummary}
			<button
				type="button"
				class="rounded-md border border-[var(--color-border)] px-3 py-1.5 text-sm font-medium hover:bg-[var(--color-bg-secondary)]"
				onclick={dashboardStore.announceSummary}
			>
				Dashboard Summary
			</button>
		{/if}
		<ConnectionStatus />
		<KeyboardShortcuts />
		<ThemeToggle />
	</div>
</header>
