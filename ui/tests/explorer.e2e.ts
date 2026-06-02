import { test, expect } from '@playwright/test';
import { mockApi } from './mocks/fixtures';

const BASE = 'http://localhost:4173/ui';

test.describe('Request Explorer (14.2)', () => {
	test('renders rows, filters, and opens the detail drawer', async ({ page }) => {
		await mockApi(page);
		await page.goto(`${BASE}/explorer`);

		// Virtualized table renders rows.
		const table = page.getByRole('grid', { name: 'Requests Explorer' });
		await expect(table).toBeVisible();
		const rows = table.locator('tbody tr');
		await expect(rows.first()).toBeVisible();
		const initialCount = await rows.count();
		expect(initialCount).toBeGreaterThan(0);

		// Apply the Model filter. The filter <select> offers fixed values
		// (llama3/mistral/qwen) that don't match the mock's '*-8b'-style ids, so
		// the filtered request returns zero rows and the empty state appears —
		// a clear, observable "table updated" signal.
		await page.locator('select[name="model"]').selectOption('qwen');
		await expect(page.getByText('No requests found')).toBeVisible();
		await expect(table).toBeHidden();

		// Clearing the filter restores the rows.
		await page.locator('select[name="model"]').selectOption('');
		await expect(page.getByRole('grid', { name: 'Requests Explorer' })).toBeVisible();
		await expect(rows.first()).toBeVisible();

		// Clicking a row opens the detail drawer.
		await rows.first().click();
		const drawer = page.getByRole('dialog', { name: 'Request Details' });
		await expect(drawer).toBeVisible();
		await expect(drawer.getByRole('heading', { name: 'Request Details' })).toBeVisible();

		// Closing returns to the table.
		await drawer.getByRole('button', { name: 'Close details' }).click();
		await expect(drawer).toBeHidden();
	});
});
