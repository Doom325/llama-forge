import { describe, it, expect, beforeEach, vi } from 'vitest';
import { preferences } from './preferences.svelte';

describe('Preferences Store', () => {
	beforeEach(() => {
		localStorage.clear();
		preferences.theme = 'system';
	});

	it('persists theme to localStorage', () => {
		preferences.setTheme('dark');
		expect(preferences.theme).toBe('dark');
		expect(localStorage.getItem('theme')).toBe('dark');
	});

	it('toggles theme properly', () => {
		preferences.setTheme('light');
		preferences.toggleTheme();
		expect(preferences.theme).toBe('dark');
		preferences.toggleTheme();
		expect(preferences.theme).toBe('light');
	});

	it('reads prefers-color-scheme initial read on system toggle', () => {
		// Mock matchMedia
		Object.defineProperty(window, 'matchMedia', {
			writable: true,
			value: vi.fn().mockImplementation((query) => ({
				matches: query === '(prefers-color-scheme: dark)',
				media: query,
				onchange: null,
				addListener: vi.fn(),
				removeListener: vi.fn(),
				addEventListener: vi.fn(),
				removeEventListener: vi.fn(),
				dispatchEvent: vi.fn()
			}))
		});

		preferences.setTheme('system');
		preferences.toggleTheme();
		expect(preferences.theme).toBe('light');
	});
});
