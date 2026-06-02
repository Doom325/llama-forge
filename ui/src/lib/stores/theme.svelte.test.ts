import { describe, it, expect, beforeEach, vi } from 'vitest';
import { preferences } from './preferences.svelte';
import { initializeTheme } from './theme.svelte';
import { flushSync } from 'svelte';

describe('Theme Store', () => {
	beforeEach(() => {
		document.documentElement.removeAttribute('data-theme');
		preferences.setTheme('system');

		Object.defineProperty(window, 'matchMedia', {
			writable: true,
			value: vi.fn().mockImplementation((query) => ({
				matches: false,
				media: query,
				onchange: null,
				addListener: vi.fn(),
				removeListener: vi.fn(),
				addEventListener: vi.fn(),
				removeEventListener: vi.fn(),
				dispatchEvent: vi.fn()
			}))
		});
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
