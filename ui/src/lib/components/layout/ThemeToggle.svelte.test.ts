import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, beforeEach } from 'vitest';
import ThemeToggle from './ThemeToggle.svelte';
import userEvent from '@testing-library/user-event';
import { preferences } from '$lib/stores/preferences.svelte';

describe('ThemeToggle', () => {
	beforeEach(() => {
		preferences.setTheme('system');
	});

	it('toggles menu open/close and renders options', async () => {
		const user = userEvent.setup();
		render(ThemeToggle);

		const toggleBtn = screen.getByRole('button', { name: 'Toggle theme' });

		// Menu initially closed
		expect(screen.queryByRole('menu')).not.toBeInTheDocument();

		// Open menu
		await user.click(toggleBtn);
		expect(screen.getByRole('menu')).toBeInTheDocument();
		expect(screen.getByRole('menuitem', { name: 'Light' })).toBeInTheDocument();
		expect(screen.getByRole('menuitem', { name: 'Dark' })).toBeInTheDocument();
		expect(screen.getByRole('menuitem', { name: 'System' })).toBeInTheDocument();

		// Close menu by clicking again
		await user.click(toggleBtn);
		expect(screen.queryByRole('menu')).not.toBeInTheDocument();
	});

	it('updates theme preference on option click', async () => {
		const user = userEvent.setup();
		render(ThemeToggle);

		await user.click(screen.getByRole('button', { name: 'Toggle theme' }));
		await user.click(screen.getByRole('menuitem', { name: 'Dark' }));

		expect(preferences.theme).toBe('dark');
		// Menu should close after selection
		expect(screen.queryByRole('menu')).not.toBeInTheDocument();
	});

	it('supports keyboard navigation', async () => {
		const user = userEvent.setup();
		render(ThemeToggle);

		await user.click(screen.getByRole('button', { name: 'Toggle theme' }));

		// The menu should be open now, we can tab to the first item (System is active)
		const systemOption = screen.getByRole('menuitem', { name: 'System' });
		systemOption.focus();

		await user.keyboard('{ArrowDown}');
		expect(document.activeElement).toHaveTextContent('Light'); // System -> Light (index 2 -> 0)

		await user.keyboard('{Enter}');
		expect(preferences.theme).toBe('light');
	});
});
