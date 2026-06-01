import { preferences } from './preferences.svelte';

export function initializeTheme() {
	if (typeof window === 'undefined') return;

	return $effect.root(() => {
		let systemPrefersDark = $state(window.matchMedia('(prefers-color-scheme: dark)').matches);

		const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
		const updateSystemTheme = (e: MediaQueryListEvent) => {
			systemPrefersDark = e.matches;
		};
		mediaQuery.addEventListener('change', updateSystemTheme);

		$effect(() => {
			let activeTheme = preferences.theme;
			if (activeTheme === 'system') {
				activeTheme = systemPrefersDark ? 'dark' : 'light';
			}

			if (activeTheme === 'dark') {
				document.documentElement.setAttribute('data-theme', 'dark');
			} else {
				document.documentElement.setAttribute('data-theme', 'light');
			}
		});

		return () => {
			mediaQuery.removeEventListener('change', updateSystemTheme);
		};
	});
}
