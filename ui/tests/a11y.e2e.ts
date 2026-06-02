import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mockApi } from './mocks/fixtures';

const BASE = 'http://localhost:4173/ui';

const PAGES: { name: string; path: string }[] = [
	{ name: 'dashboard', path: '/' },
	{ name: 'explorer', path: '/explorer' },
	{ name: 'config', path: '/config/global' },
	{ name: 'playground', path: '/playground' }
];

const THEMES: ('light' | 'dark')[] = ['light', 'dark'];

async function runAxe(page: Page) {
	return new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
}

for (const theme of THEMES) {
	for (const { name, path } of PAGES) {
		test(`a11y: ${name} (${theme}) has no WCAG 2 A/AA violations`, async ({ page }) => {
			await mockApi(page);
			// Drive the theme via the localStorage key the preferences store reads.
			await page.addInitScript((t) => {
				localStorage.setItem('theme', t);
			}, theme);
			await page.goto(`${BASE}${path}`);
			// Let the route's primary content settle (queries resolve, table renders).
			await page.waitForLoadState('networkidle').catch(() => {});

			const results = await runAxe(page);
			expect(results.violations, JSON.stringify(results.violations, null, 2)).toEqual([]);
		});
	}
}
