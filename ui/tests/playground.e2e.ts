import { test, expect } from '@playwright/test';
import { mockApi } from './mocks/fixtures';

const BASE = 'http://localhost:4173/ui';

test.describe('Playground (14.4)', () => {
	test('streams an assistant response and renders it', async ({ page }) => {
		await mockApi(page);
		await page.goto(`${BASE}/playground`);

		const input = page.locator('#chat-input');
		await expect(input).toBeVisible();
		await input.fill('Tell me something');

		await page.getByRole('button', { name: 'Send', exact: true }).click();

		// The mocked SSE stream renders a short markdown reply. Assert distinctive
		// content from the streamed assistant message appears.
		const log = page.getByRole('log', { name: 'Conversation' });
		await expect(log.getByText('streamed', { exact: false })).toBeVisible();
		await expect(log.getByText('second point')).toBeVisible();
	});
});
