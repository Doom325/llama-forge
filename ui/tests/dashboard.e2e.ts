import { test, expect } from '@playwright/test';
import { mockApi, servers } from './mocks/fixtures';

const BASE = 'http://localhost:4173/ui';

test.describe('Dashboard (14.1)', () => {
	test('renders server cards and navigates to detail on click', async ({ page }) => {
		await mockApi(page);
		await page.goto(`${BASE}/`);

		// Server cards render as grid cells, one per mocked server.
		const cards = page.getByRole('gridcell');
		await expect(cards.first()).toBeVisible();
		await expect(cards).toHaveCount(servers.length);

		// Clicking a card navigates to its detail route.
		const first = servers[0]!;
		await page.getByRole('gridcell', { name: `Server ${first.name}` }).click();
		await expect(page).toHaveURL(new RegExp(`/ui/servers/${first.id}$`));
		// The detail page's H1 is the server's name.
		await expect(page.getByRole('heading', { level: 1, name: first.name })).toBeVisible();
	});
});
