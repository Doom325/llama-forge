<script lang="ts">
	import { page } from '$app/state';
	import { preferences } from '$lib/stores/preferences.svelte';
	import { prefetchRouteData } from '$lib/api/queries';
	import { base } from '$app/paths';

	const links = [
		{ href: '/', label: 'Dashboard', icon: '📊', routeKey: 'dashboard' },
		{ href: '/queue', label: 'Queue', icon: '☰', routeKey: 'queue' },
		{ href: '/explorer', label: 'Explorer', icon: '🔍', routeKey: 'explorer' },
		{ href: '/playground', label: 'Playground', icon: '⚡', routeKey: 'playground' },
		{ href: '/models', label: 'Models', icon: '🧩', routeKey: 'models' },
		{ href: '/config/servers', label: 'Servers Config', icon: '⚙️', routeKey: 'config/servers' },
		{ href: '/config/priority', label: 'Priority Config', icon: '⚖️', routeKey: 'config/priority' }
	];

	function handleKeydown(event: KeyboardEvent, index: number) {
		if (event.key === 'ArrowDown') {
			event.preventDefault();
			const next = (index + 1) % links.length;
			const el = document.getElementById(`nav-link-${next}`);
			el?.focus();
		} else if (event.key === 'ArrowUp') {
			event.preventDefault();
			const prev = (index - 1 + links.length) % links.length;
			const el = document.getElementById(`nav-link-${prev}`);
			el?.focus();
		}
	}
</script>

{#if !preferences.sidebarCollapsed}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed inset-0 z-40 bg-black/50 md:hidden"
		onclick={() => preferences.toggleSidebar()}
	></div>
{/if}

<aside
	id="mobile-sidebar"
	class="skeleton-sidebar fixed inset-y-0 left-0 z-50 flex transform flex-col border-r border-[var(--color-border)] bg-[var(--color-bg-primary)] transition-transform duration-200 md:static md:translate-x-0 {preferences.sidebarCollapsed
		? '-translate-x-full md:w-[var(--sidebar-collapsed-width)]'
		: 'w-[var(--sidebar-width)] -translate-x-full md:translate-x-0'}"
>
	<div
		class="flex h-[var(--header-height)] items-center justify-between border-b border-[var(--color-border)] px-4"
	>
		{#if !preferences.sidebarCollapsed}
			<span class="text-lg font-bold">LlamaForge</span>
		{/if}
		<button
			type="button"
			class="rounded-md p-1.5 hover:bg-[var(--color-bg-secondary)]"
			aria-label={preferences.sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
			aria-expanded={!preferences.sidebarCollapsed}
			onclick={() => preferences.toggleSidebar()}
		>
			<span aria-hidden="true">{preferences.sidebarCollapsed ? '→' : '←'}</span>
		</button>
	</div>

	<nav aria-label="Main Navigation" class="flex-1 overflow-y-auto py-4">
		<ul class="space-y-1 px-2">
			{#each links as link, index (link.routeKey)}
				{@const isActive =
					page.url.pathname === base + link.href ||
					(link.href !== '/' && page.url.pathname.startsWith(base + link.href))}
				<li>
					<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
					<a
						id="nav-link-{index}"
						href="{base}{link.href}"
						class="flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors {isActive
							? 'bg-[var(--color-nav-active-bg)] text-[var(--color-nav-active-text)]'
							: 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)] hover:text-[var(--color-text-primary)]'}"
						aria-current={isActive ? 'page' : undefined}
						onmouseenter={() => prefetchRouteData(link.routeKey)}
						onkeydown={(e) => handleKeydown(e, index)}
					>
						<span aria-hidden="true" class="text-base">{link.icon}</span>
						{#if !preferences.sidebarCollapsed}
							<span>{link.label}</span>
						{/if}
					</a>
				</li>
			{/each}
		</ul>
	</nav>
</aside>
