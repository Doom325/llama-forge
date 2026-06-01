import { browser } from '$app/environment';
import { preferences } from './preferences.svelte';

export function initializeTheme() {
	if (!browser) return;

	$effect.root(() => {
		$effect(() => {
			let activeTheme = preferences.theme;
			if (activeTheme === 'system') {
				activeTheme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
			}

			if (activeTheme === 'dark') {
				document.documentElement.setAttribute('data-theme', 'dark');
			} else {
				document.documentElement.setAttribute('data-theme', 'light');
			}
		});
	});
}
