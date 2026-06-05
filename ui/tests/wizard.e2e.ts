import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mockSetupApi } from './mocks/setup-fixtures';

const BASE = 'http://localhost:4173/ui';

// Full wizard walk: Welcome → Hardware → Binary → Models → Configure → Review →
// Save & Launch → running (08_SETUP_WIZARD.md §2.1, PLAN 13.7). All /api/setup/*
// responses are mocked to the happy path.
test('wizard happy path reaches the running completion screen', async ({ page }) => {
	await mockSetupApi(page);
	await page.goto(`${BASE}/setup`);
	await page.waitForLoadState('networkidle').catch(() => {});

	await page.getByRole('button', { name: 'Start Setup' }).click();

	// Hardware
	await expect(page.getByText('NVIDIA RTX 4090')).toBeVisible({ timeout: 15_000 });
	await page.getByRole('button', { name: 'Next' }).click();

	// Binary (found via the mock)
	await expect(page.getByText('Found llama-server')).toBeVisible();
	await page.getByRole('button', { name: 'Next' }).click();

	// Models — select the discovered model to satisfy the step guard
	await expect(page.getByText('llama-3-8b-Q4_K_M.gguf')).toBeVisible();
	await page.getByLabel('Select llama-3-8b-Q4_K_M.gguf').check();
	await page.getByRole('button', { name: 'Next' }).click();

	// Configure (recommendation card)
	await expect(page.getByLabel('Alias')).toBeVisible();
	await page.getByRole('button', { name: 'Next' }).click();

	// Review → save → validate → status poll → success
	await expect(page.getByRole('button', { name: 'Save & Launch' })).toBeVisible();
	await page.getByRole('button', { name: 'Save & Launch' }).click();
	await expect(page.getByRole('button', { name: 'Go to Dashboard' })).toBeVisible({
		timeout: 15_000
	});
});

// Focus management (PLAN 13.15 keyboard a11y): navigating to a step moves focus
// to that step's heading.
test('navigating a step moves focus to its heading', async ({ page }) => {
	await mockSetupApi(page);
	await page.goto(`${BASE}/setup`);
	await page.waitForLoadState('networkidle').catch(() => {});
	await page.getByRole('button', { name: 'Start Setup' }).click();
	await expect(page.getByText('NVIDIA RTX 4090')).toBeVisible({ timeout: 15_000 });
	const focusedTag = await page.evaluate(() => document.activeElement?.tagName);
	expect(focusedTag).toBe('H1');
});

async function runAxe(page: Page) {
	return new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
}

// axe-core on the wizard steps (PLAN 13.17).
for (const { name, path } of [
	{ name: 'welcome', path: '/setup' },
	{ name: 'hardware', path: '/setup/hardware' }
]) {
	test(`a11y: setup ${name} has no WCAG 2 A/AA violations`, async ({ page }) => {
		await mockSetupApi(page);
		await page.goto(`${BASE}${path}`);
		await page.waitForLoadState('networkidle').catch(() => {});
		const results = await runAxe(page);
		expect(results.violations, JSON.stringify(results.violations, null, 2)).toEqual([]);
	});
}
