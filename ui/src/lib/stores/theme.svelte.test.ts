import { describe, it, expect, beforeEach } from 'vitest';
import { preferences } from './preferences.svelte';
import { initializeTheme } from './theme.svelte';
import { flushSync } from 'svelte';

describe('Theme Store', () => {
	beforeEach(() => {
		document.documentElement.removeAttribute('data-theme');
		preferences.setTheme('system');
	});

	it('applies data-theme attribute on theme change', () => {
		initializeTheme();

		preferences.setTheme('dark');
		flushSync();
		expect(document.documentElement.getAttribute('data-theme')).toBe('dark');

		preferences.setTheme('light');
		flushSync();
		expect(document.documentElement.getAttribute('data-theme')).toBe('light');
	});
});
