<script lang="ts">
	import { page } from '$app/state';
	import { base } from '$app/paths';
	import type { Snippet } from 'svelte';

	let { children }: { children: Snippet } = $props();

	const tabs = [
		{ href: '/config/servers', label: 'Server config' },
		{ href: '/config/priority', label: 'Priority rules' },
		{ href: '/config/global', label: 'Global settings' }
	];
</script>

<div class="mx-auto max-w-[1400px] space-y-6">
	<div role="tablist" class="flex gap-1 border-b border-[var(--color-border)]">
		{#each tabs as tab (tab.href)}
			{@const isActive = page.url.pathname === base + tab.href}
			<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
			<a
				role="tab"
				href="{base}{tab.href}"
				aria-selected={isActive}
				class="border-b-2 px-4 py-2 text-sm font-medium transition-colors {isActive
					? 'border-[var(--color-focus-ring)] text-[var(--color-text-primary)]'
					: 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'}"
			>
				{tab.label}
			</a>
		{/each}
	</div>

	{@render children()}
</div>
