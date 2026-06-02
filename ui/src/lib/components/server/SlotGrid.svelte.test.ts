import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import SlotGrid from './SlotGrid.svelte';

describe('SlotGrid', () => {
	it('renders slot grid correctly', () => {
		const slots: import('./SlotGrid.svelte').SlotState[] = [
			{ id: '1', status: 'idle' },
			{ id: '2', status: 'idle' },
			{ id: '3', status: 'idle' },
			{ id: '4', status: 'idle' },
			{ id: '5', status: 'idle' },
			{ id: '6', status: 'processing', model: 'llama3', requestId: 'req-1' },
			{ id: '7', status: 'processing', model: 'llama3', requestId: 'req-2' },
			{ id: '8', status: 'loading', model: 'mistral' }
		];

		render(SlotGrid, { slots, serverName: 'TestServer' });

		expect(screen.getByText('Slots (2 Active / 8 Total)')).toBeInTheDocument();

		// Idle slots should map to healthy badges
		expect(screen.getAllByLabelText('Status: Healthy').length).toBe(5);

		// Processing slots should map to pending badges
		expect(screen.getAllByLabelText('Status: Pending').length).toBe(2);

		// Loading slots should map to reconnecting badges
		expect(screen.getAllByLabelText('Status: Reconnecting').length).toBe(1);

		expect(screen.getByText('req-1')).toBeInTheDocument();
		expect(screen.getByText('mistral')).toBeInTheDocument();
	});

	it('renders empty state', () => {
		render(SlotGrid, { slots: [], serverName: 'TestServer' });
		expect(screen.getByText('No slots configured for TestServer')).toBeInTheDocument();
	});
});
