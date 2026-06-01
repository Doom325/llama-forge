<script lang="ts">
	import { preferences } from '$lib/stores/preferences.svelte';

	let open = $state(false);
	let buttonRef: HTMLButtonElement | null = $state(null);
	let menuRef: HTMLDivElement | null = $state(null);

	function handleKeydown(event: KeyboardEvent, index: number, max: number) {
		if (event.key === 'ArrowDown') {
			event.preventDefault();
			const next = (index + 1) % max;
			const el = document.getElementById(`theme-option-${next}`);
			el?.focus();
		} else if (event.key === 'ArrowUp') {
			event.preventDefault();
			const prev = (index - 1 + max) % max;
			const el = document.getElementById(`theme-option-${prev}`);
			el?.focus();
		} else if (event.key === 'Enter' || event.key === ' ') {
			event.preventDefault();
			const theme = ['light', 'dark', 'system'][index] as 'light' | 'dark' | 'system';
			preferences.setTheme(theme);
			closeMenu();
		} else if (event.key === 'Escape') {
			closeMenu();
		}
	}

	function toggleMenu() {
		open = !open;
		if (open) {
			setTimeout(() => {
				const firstOption = document.getElementById('theme-option-0');
				firstOption?.focus();
			}, 0);
		}
	}

	function closeMenu() {
		open = false;
		buttonRef?.focus();
	}

	function handleWindowClick(e: MouseEvent) {
		if (
			open &&
			menuRef &&
			buttonRef &&
			!menuRef.contains(e.target as Node) &&
			!buttonRef.contains(e.target as Node)
		) {
			closeMenu();
		}
	}
</script>

<svelte:window onclick={handleWindowClick} />

<div class="relative inline-block text-left">
	<button
		bind:this={buttonRef}
		id="theme-menu-button"
		type="button"
		class="inline-flex h-9 w-9 items-center justify-center rounded-md border border-[var(--color-border)] hover:bg-[var(--color-bg-secondary)] focus-visible:outline-2 focus-visible:outline-offset-2"
		onclick={toggleMenu}
		aria-expanded={open}
		aria-haspopup="menu"
		aria-label="Toggle theme"
	>
		{#if preferences.theme === 'light'}
			<span aria-hidden="true">☀️</span>
		{:else if preferences.theme === 'dark'}
			<span aria-hidden="true">🌙</span>
		{:else}
			<span aria-hidden="true">⚙️</span>
		{/if}
	</button>

	{#if open}
		<div
			bind:this={menuRef}
			class="absolute right-0 z-50 mt-2 w-32 origin-top-right rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] shadow-lg focus:outline-none"
			role="menu"
			aria-orientation="vertical"
			aria-labelledby="theme-menu-button"
			tabindex="-1"
		>
			<div class="py-1" role="none">
				{#each ['light', 'dark', 'system'] as theme, index (theme)}
					<button
						id="theme-option-{index}"
						class="block w-full px-4 py-2 text-left text-sm text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)] focus:bg-[var(--color-bg-secondary)] focus:outline-none"
						role="menuitem"
						tabindex={index === 0 ? 0 : -1}
						onclick={() => {
							preferences.setTheme(theme as 'light' | 'dark' | 'system');
							closeMenu();
						}}
						onkeydown={(e) => handleKeydown(e, index, 3)}
					>
						{theme.charAt(0).toUpperCase() + theme.slice(1)}
					</button>
				{/each}
			</div>
		</div>
	{/if}
</div>
