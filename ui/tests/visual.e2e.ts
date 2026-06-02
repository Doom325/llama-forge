import { test, expect, type Page, type Locator } from '@playwright/test';
import { mockApi } from './mocks/fixtures';

const BASE = 'http://localhost:4173/ui';

const PAGES: { name: string; path: string }[] = [
	{ name: 'dashboard', path: '/' },
	{ name: 'explorer', path: '/explorer' },
	{ name: 'config', path: '/config/global' },
	{ name: 'playground', path: '/playground' }
];

const THEMES: ('light' | 'dark')[] = ['light', 'dark'];

// Regions whose content is inherently volatile across runs and would cause
// false-positive screenshot diffs: live timestamps, computed metric values,
// and the connection-status chip (depends on SSE timing).
function volatileMasks(page: Page): Locator[] {
	return [
		// Connection / "Reconnecting" status chip in the header.
		page.locator('header [aria-live="polite"]'),
		// Toast notifications (success/info popups).
		page.locator('[role="status"].fixed'),
		// Explorer table timestamp column cells (2nd column).
		page.locator('table tbody tr td:nth-child(2)'),
		// Metric value tiles on the dashboard (throughput number).
		page.locator('.text-3xl')
	];
}

for (const theme of THEMES) {
	for (const { name, path } of PAGES) {
		test(`visual: ${name} (${theme})`, async ({ page }) => {
			await mockApi(page);
			await page.addInitScript((t) => {
				localStorage.setItem('theme', t);
			}, theme);
			await page.goto(`${BASE}${path}`);
			await page.waitForLoadState('networkidle').catch(() => {});
			// Disable animations/transitions for deterministic pixels.
			await page.addStyleTag({
				content:
					'*, *::before, *::after { transition: none !important; animation: none !important; }'
			});

			await expect(page).toHaveScreenshot(`${name}-${theme}.png`, {
				fullPage: true,
				mask: volatileMasks(page),
				// ~0.2% tolerance for sub-pixel/AA jitter between runs.
				maxDiffPixelRatio: 0.002,
				animations: 'disabled'
			});
		});
	}
}
