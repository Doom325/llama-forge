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

// Navigation resilience (PLAN 13.14): going back to Hardware keeps its cached
// data, and re-entering invalidates the downstream Configure overrides.
test('back to Hardware keeps cache; Configure overrides are invalidated', async ({ page }) => {
	await mockSetupApi(page);
	await page.goto(`${BASE}/setup`);
	await page.waitForLoadState('networkidle').catch(() => {});

	await page.getByRole('button', { name: 'Start Setup' }).click();
	await expect(page.getByText('NVIDIA RTX 4090')).toBeVisible({ timeout: 15_000 });
	await page.getByRole('button', { name: 'Next' }).click();
	await expect(page.getByText('Found llama-server')).toBeVisible();
	await page.getByRole('button', { name: 'Next' }).click();
	await page.getByLabel('Select llama-3-8b-Q4_K_M.gguf').check();
	await page.getByRole('button', { name: 'Next' }).click();

	// At Configure: capture the recommended alias, then override it.
	const alias = page.getByLabel('Alias');
	await expect(alias).toBeVisible();
	const recommended = await alias.inputValue();
	await alias.fill('my-custom-alias');

	// Back to Hardware (configure → models → binary → hardware).
	await page.getByRole('button', { name: 'Back' }).click();
	await page.getByRole('button', { name: 'Back' }).click();
	await page.getByRole('button', { name: 'Back' }).click();
	// Hardware data is still cached — the card shows without re-detecting.
	await expect(page.getByText('NVIDIA RTX 4090')).toBeVisible();

	// Forward again to Configure.
	await page.getByRole('button', { name: 'Next' }).click();
	await expect(page.getByText('Found llama-server')).toBeVisible();
	await page.getByRole('button', { name: 'Next' }).click();
	await page.getByLabel('Select llama-3-8b-Q4_K_M.gguf').check();
	await page.getByRole('button', { name: 'Next' }).click();

	// The override was invalidated on re-entry → alias is back to the default.
	await expect(page.getByLabel('Alias')).toHaveValue(recommended);
});

// Hardware detection failure → manual entry (PLAN 13.10): the manually-entered
// hardware lands in the cache the Recommend step reads (the cards re-render from
// it), so a 500 from detection doesn't block setup.
test('hardware detection 500 → manual entry populates the hardware cache', async ({ page }) => {
	await mockSetupApi(page, { hardwareError: true });
	await page.goto(`${BASE}/setup/hardware`);
	await page.waitForLoadState('networkidle').catch(() => {});

	await expect(page.getByText('Hardware detection failed.')).toBeVisible();
	await page.getByRole('button', { name: /Enter hardware manually/ }).click();
	await page.getByLabel(/GPU name/).fill('Manual GPU X');
	await page.getByLabel(/GPU VRAM/).fill('8000');
	await page.getByLabel('CPU cores').fill('8');
	await page.getByLabel(/System RAM/).fill('16000');
	await page.getByRole('button', { name: 'Use this hardware' }).click();

	// applyManual writes ['setup','hardware'] → the cards re-render from the cache.
	await expect(page.getByText('Manual GPU X')).toBeVisible();
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
