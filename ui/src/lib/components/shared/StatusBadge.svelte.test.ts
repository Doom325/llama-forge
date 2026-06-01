import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import StatusBadge from './StatusBadge.svelte';

describe('StatusBadge', () => {
	it('renders healthy status correctly', () => {
		render(StatusBadge, { status: 'healthy' });
		const badge = screen.getByText('Healthy');
		expect(badge).toBeInTheDocument();
		expect(screen.getByText('✓')).toBeInTheDocument();
		const container = badge.parentElement;
		expect(container).toHaveAttribute('aria-label', 'Status: Healthy');
		expect(container).toHaveClass('text-[var(--color-status-healthy)]');
	});

	it('renders degraded status correctly', () => {
		render(StatusBadge, { status: 'degraded' });
		expect(screen.getByText('Degraded')).toBeInTheDocument();
		expect(screen.getByText('⚠')).toBeInTheDocument();
	});

	it('renders failed status correctly', () => {
		render(StatusBadge, { status: 'failed' });
		expect(screen.getByText('Failed')).toBeInTheDocument();
		expect(screen.getByText('✕')).toBeInTheDocument();
	});

	it('renders pending status correctly', () => {
		render(StatusBadge, { status: 'pending' });
		expect(screen.getByText('Pending')).toBeInTheDocument();
		expect(screen.getByText('⟳')).toBeInTheDocument();
	});
});
