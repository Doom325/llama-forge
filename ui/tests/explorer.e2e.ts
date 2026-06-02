import { test, expect } from '@playwright/test';

test('explorer page renders rows', async ({ page }) => {
	page.on('console', (msg) => console.log('BROWSER CONSOLE:', msg.text()));

	// Mock the API request
	await page.route('**/api/requests*', async (route) => {
		await route.fulfill({
			status: 200,
			contentType: 'application/json',
			body: JSON.stringify([
				{
					id: '1',
					timestamp: '2023-01-01T12:00:00Z',
					model: 'llama3',
					taskType: 'chat',
					duration: 1500,
					status: 'success'
				},
				{
					id: '2',
					timestamp: '2023-01-01T12:01:00Z',
					model: 'mistral',
					taskType: 'completion',
					duration: 500,
					status: 'success'
				}
			])
		});
	});

	await page.goto('http://localhost:4173/ui/explorer');

	// Wait for the grid to appear and not be animating pulse
	try {
		await page.waitForSelector('table', { timeout: 5000 });
	} catch (e) {
		const html = await page.content();
		console.log('PAGE HTML:', html);
		throw e;
	}
	const rows = await page.locator('table tbody tr').count();
	console.log(`Found ${rows} rows`);

	expect(rows).toBeGreaterThan(0);
});
