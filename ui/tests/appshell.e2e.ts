import { test, expect } from '@playwright/test';

test('AppShell layout geometry', async ({ page }) => {
	await page.goto('/');

	// The skeleton wrapper hides on hydration, ensure it's gone
	const skeleton = page.locator('#pre-hydration-skeleton');
	await expect(skeleton).toBeHidden();

	// Check main app layout
	const sidebar = page.locator('.app-shell > aside');
	const header = page.locator('.app-shell > header');
	const main = page.locator('.app-shell > main');

	await expect(sidebar).toBeVisible();
	await expect(header).toBeVisible();
	await expect(main).toBeVisible();

	const sidebarBox = await sidebar.boundingBox();
	const headerBox = await header.boundingBox();
	const mainBox = await main.boundingBox();

	expect(sidebarBox?.x).toBe(0);
	expect(sidebarBox?.y).toBe(0);
	expect(sidebarBox?.width).toBe(240); // Matches --sidebar-width
	expect(sidebarBox?.height).toBeGreaterThan(500); // Full height

	expect(headerBox?.x).toBe(240); // Right of sidebar
	expect(headerBox?.y).toBe(0);
	expect(headerBox?.height).toBe(56); // Matches --header-height

	expect(mainBox?.x).toBe(240); // Right of sidebar
	expect(mainBox?.y).toBe(56); // Below header
});
