import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, beforeEach } from 'vitest';
import AppShellWrapper from '../testing/AppShellWrapper.svelte';
import { preferences } from '$lib/stores/preferences.svelte';

describe('AppShell', () => {
	beforeEach(() => {
		preferences.sidebarCollapsed = false;
	});

	it('renders skip link and main layout grid', () => {
		const { container } = render(AppShellWrapper);

		const skipLink = screen.getByText('Skip to main content');
		expect(skipLink).toBeInTheDocument();
		expect(skipLink).toHaveAttribute('href', '#main-content');
		expect(skipLink).toHaveClass('sr-only'); // Visually hidden until focused

		const appDiv = container.querySelector('.app-shell');
		expect(appDiv).toBeInTheDocument();
		expect(appDiv).toHaveClass('grid');
		expect(appDiv).not.toHaveClass('collapsed');

		expect(screen.getByText('Test content')).toBeInTheDocument();
	});

	it('adds collapsed class when preferences sidebar is collapsed', () => {
		preferences.sidebarCollapsed = true;
		const { container } = render(AppShellWrapper);

		const appDiv = container.querySelector('.app-shell');
		expect(appDiv).toHaveClass('collapsed');
	});
});
