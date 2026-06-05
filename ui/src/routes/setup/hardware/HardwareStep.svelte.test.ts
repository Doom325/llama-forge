import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';

// Mutable mock query result, set per test before rendering.
const mocks = vi.hoisted(() => ({
	hw: { isPending: true, isError: false, data: undefined, refetch: vi.fn() } as Record<
		string,
		unknown
	>
}));

vi.mock('$lib/stores/setup.svelte', () => ({
	useSetupStore: () => ({ sessionId: 's' })
}));
vi.mock('$lib/api/queries', () => ({
	createHardwareQuery: () => mocks.hw
}));
vi.mock('$lib/api/queryClient', () => ({
	queryClient: { setQueryData: vi.fn() }
}));

import HardwareStep from './+page.svelte';

describe('HardwareStep', () => {
	it('renders the heading while detection is pending (no cards/error)', () => {
		mocks.hw = { isPending: true, isError: false, data: undefined, refetch: vi.fn() };
		render(HardwareStep);
		expect(screen.getByRole('heading', { name: 'Hardware' })).toBeInTheDocument();
		expect(screen.queryByText('Hardware detection failed.')).toBeNull();
	});

	it('shows an error banner with retry on failure', () => {
		mocks.hw = { isPending: false, isError: true, data: undefined, refetch: vi.fn() };
		render(HardwareStep);
		expect(screen.getByText('Hardware detection failed.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: /Enter hardware manually/ })).toBeInTheDocument();
	});

	it('renders hardware cards on success', () => {
		mocks.hw = {
			isPending: false,
			isError: false,
			refetch: vi.fn(),
			data: {
				gpus: [{ index: 0, name: 'RTX 4090', vram_mb: 24576 }],
				cpu: { name: 'Ryzen 9', cores_physical: 8, cores_logical: 16 },
				ram_mb: 32768,
				recommended_backend: 'cuda'
			}
		};
		render(HardwareStep);
		expect(screen.getByText('RTX 4090')).toBeInTheDocument();
		expect(screen.getByText('Ryzen 9')).toBeInTheDocument();
		expect(screen.getByText('System RAM')).toBeInTheDocument();
		expect(screen.getByText('CUDA')).toBeInTheDocument();
	});
});
