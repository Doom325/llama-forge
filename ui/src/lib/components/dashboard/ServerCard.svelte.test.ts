import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ServerCard from './ServerCard.svelte';
import userEvent from '@testing-library/user-event';
import type { ServerState } from '$lib/api/types';

describe('ServerCard', () => {
	const mockServer: ServerState = {
		id: 's-123',
		name: 'Mistral-7B',
		status: 'healthy',
		uptime: 3600
	};

	it('renders correctly with metrics', () => {
		render(ServerCard, {
			server: mockServer,
			metrics: {
				throughput: 45.2,
				latency: { p50: 10, p90: 20, p99: 30 },
				errorRate: 0,
				slots: { total: 4, idle: 2, processing: 2, loading: 0 }
			},
			availableActions: ['restart', 'stop'],
			onAction: vi.fn()
		});

		expect(screen.getByText('Mistral-7B')).toBeInTheDocument();
		expect(screen.getByText('ID: s-123')).toBeInTheDocument();
		expect(screen.getByText('45.2 tok/s')).toBeInTheDocument();
		expect(screen.getByText('2 / 2 (Total: 4)')).toBeInTheDocument();
		expect(screen.getByText('60m')).toBeInTheDocument();
	});

	it('renders availableActions correctly', () => {
		const { rerender } = render(ServerCard, {
			server: mockServer,
			availableActions: ['restart', 'stop'],
			onAction: vi.fn()
		});

		expect(screen.getByRole('button', { name: 'Restart' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Start' })).not.toBeInTheDocument();

		rerender({ server: mockServer, availableActions: ['start'], onAction: vi.fn() });

		expect(screen.getByRole('button', { name: 'Start' })).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Restart' })).not.toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Stop' })).not.toBeInTheDocument();
	});

	it('action click does not navigate', async () => {
		const onAction = vi.fn();
		const user = userEvent.setup();

		render(ServerCard, {
			server: mockServer,
			availableActions: ['restart'],
			onAction
		});

		const preventDefaultSpy = vi.spyOn(Event.prototype, 'preventDefault');
		const stopPropagationSpy = vi.spyOn(Event.prototype, 'stopPropagation');

		await user.click(screen.getByRole('button', { name: 'Restart' }));

		expect(onAction).toHaveBeenCalledWith('s-123', 'restart');
		expect(preventDefaultSpy).toHaveBeenCalled();
		expect(stopPropagationSpy).toHaveBeenCalled();
	});
});
