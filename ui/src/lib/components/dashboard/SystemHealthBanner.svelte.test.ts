import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import SystemHealthBanner from './SystemHealthBanner.svelte';

describe('SystemHealthBanner', () => {
	it('does not render when healthy', () => {
		const { container } = render(SystemHealthBanner, {
			healthStatus: 'healthy',
			serverCounts: { degraded: 0, failed: 0, total: 2 }
		});

		expect(container.textContent?.trim()).toBe('');
	});

	it('renders warning when degraded', () => {
		render(SystemHealthBanner, {
			healthStatus: 'degraded',
			serverCounts: { degraded: 1, failed: 0, total: 2 }
		});

		expect(
			screen.getByText('1 of 2 servers are operating in a degraded state.')
		).toBeInTheDocument();
		expect(screen.getByRole('status')).toBeInTheDocument();
	});

	it('renders error when failed', () => {
		render(SystemHealthBanner, {
			healthStatus: 'failed',
			serverCounts: { degraded: 0, failed: 1, total: 2 }
		});

		expect(
			screen.getByText('1 of 2 servers are in a failed state. System capacity is severely reduced.')
		).toBeInTheDocument();
		expect(screen.getByRole('alert')).toBeInTheDocument();
	});
});
