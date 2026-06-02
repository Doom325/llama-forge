import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import SidebarWrapper from '../testing/SidebarWrapper.svelte';
import userEvent from '@testing-library/user-event';
import { preferences } from '$lib/stores/preferences.svelte';

// Mock app modules
vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/ui/queue')
	}
}));

vi.mock('$app/paths', () => ({
	base: '/ui'
}));

describe('Sidebar', () => {
	beforeEach(() => {
		preferences.sidebarCollapsed = false;
	});

	it('renders navigation links and ARIA attributes', () => {
		render(SidebarWrapper);

		expect(screen.getByRole('navigation')).toBeInTheDocument();

		const queueLink = screen.getByRole('link', { name: /Queue/i });
		expect(queueLink).toBeInTheDocument();
		// In tests with adapter-static configured for /ui base, this will be /ui/queue or /queue depending on Vitest resolution.
		expect(queueLink.getAttribute('href')).toMatch(/\/queue$/);

		// It's the active route, should have aria-current
		expect(queueLink).toHaveAttribute('aria-current', 'page');

		const dashboardLink = screen.getByRole('link', { name: /Dashboard/i });
		expect(dashboardLink).not.toHaveAttribute('aria-current');
	});

	it('toggles sidebar collapse state on button click', async () => {
		const user = userEvent.setup();
		render(SidebarWrapper);

		const toggleBtn = screen.getByRole('button', { name: /Collapse sidebar/i });
		await user.click(toggleBtn);

		expect(preferences.sidebarCollapsed).toBe(true);

		// The text label should be hidden when collapsed (Svelte removes the DOM element in our #if block)
		expect(screen.queryByText('Dashboard', { selector: 'span' })).not.toBeInTheDocument();
	});

	it('navigates via arrow keys', async () => {
		const user = userEvent.setup();
		const { container } = render(SidebarWrapper);

		const dashboardLink = container.querySelector('#nav-link-0') as HTMLElement;
		dashboardLink.focus();

		await user.keyboard('{ArrowDown}');
		const queueLink = container.querySelector('#nav-link-1') as HTMLElement;
		expect(document.activeElement).toBe(queueLink);

		await user.keyboard('{ArrowUp}');
		expect(document.activeElement).toBe(dashboardLink);
	});
});
