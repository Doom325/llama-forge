import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import QueueTable from './QueueTable.svelte';
import type { QueueItem } from '$lib/api/types';
import { SvelteSet } from 'svelte/reactivity';

describe('QueueTable', () => {
	const mockItems: QueueItem[] = [
		{
			id: 'req-1',
			position: 1,
			model: 'llama3',
			requestor: 'admin',
			status: 'processing',
			waitTime: 120
		},
		{
			id: 'req-2',
			position: 2,
			model: 'llama3',
			requestor: 'user1',
			status: 'queued',
			waitTime: 60
		},
		{
			id: 'req-3',
			position: 3,
			model: 'mistral',
			requestor: 'user2',
			status: 'queued',
			waitTime: 30
		},
		{
			id: 'req-4',
			position: 4,
			model: 'mistral',
			requestor: 'admin',
			status: 'queued',
			waitTime: 15
		},
		{ id: 'req-5', position: 5, model: 'qwen', requestor: 'user3', status: 'queued', waitTime: 5 }
	];

	it('renders 5 items correctly', () => {
		render(QueueTable, { items: mockItems, onCancel: vi.fn() });

		// 5 rows + 1 header row
		expect(screen.getAllByRole('row').length).toBe(6);

		expect(screen.getByText('req-1')).toBeInTheDocument();
		expect(screen.getByText('req-5')).toBeInTheDocument();
		expect(screen.getAllByRole('button', { name: /Cancel request req-/ }).length).toBe(5);
	});

	it('sort header has aria-sort', () => {
		render(QueueTable, { items: mockItems, onCancel: vi.fn() });

		const posHeader = screen.getByRole('columnheader', { name: 'Position' });
		expect(posHeader).toHaveAttribute('aria-sort', 'ascending');
	});

	it('reflects cancelling state optimistically', () => {
		const pendingCancels = new SvelteSet(['req-2']);
		render(QueueTable, { items: mockItems, pendingCancels, onCancel: vi.fn() });

		const cancellingBtn = screen.getByRole('button', { name: 'Cancel request req-2' });
		expect(cancellingBtn).toHaveTextContent('Cancelling...');
		expect(cancellingBtn).toBeDisabled();

		// Badge logic
		expect(screen.getAllByText('Cancelling...').length).toBeGreaterThan(0);
	});
});
