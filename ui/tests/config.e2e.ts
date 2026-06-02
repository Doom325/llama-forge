import { test, expect } from '@playwright/test';
import { mockApi } from './mocks/fixtures';

const BASE = 'http://localhost:4173/ui';

test.describe('Global Config (14.3)', () => {
	test('edits a field, reviews the diff, and saves with a success toast', async ({ page }) => {
		await mockApi(page);
		await page.goto(`${BASE}/config/global`);

		const httpPort = page.locator('#http_port');
		await expect(httpPort).toBeVisible();

		// Change a field — this makes the form dirty and reveals "Review Changes".
		await httpPort.fill('9090');
		await httpPort.blur();

		const review = page.getByRole('button', { name: 'Review Changes' });
		await expect(review).toBeVisible();
		await review.click();

		// Diff modal opens and shows the pending change.
		const dialog = page.getByRole('dialog', { name: 'Review Changes' });
		await expect(dialog).toBeVisible();
		await expect(dialog.getByText('http_port')).toBeVisible();
		await dialog.getByRole('button', { name: 'Close' }).click();
		await expect(dialog).toBeHidden();

		// Save → success toast.
		await page.getByRole('button', { name: 'Save' }).click();
		await expect(page.getByText('Configuration saved successfully')).toBeVisible();
	});
});
