import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
	scan: { isPending: true, mutate: vi.fn() } as Record<string, unknown>
}));

vi.mock('$lib/stores/setup.svelte', () => ({
	useSetupStore: () => ({ sessionId: 's', selectedModelIds: new Set<string>() })
}));
vi.mock('$lib/api/queries', () => ({
	createModelScanMutation: () => mocks.scan
}));

import ModelStep from './+page.svelte';

const model = {
	id: 'm1',
	path: '/a.gguf',
	filename: 'a.gguf',
	size_bytes: 1024,
	quantization_hint: 'Q4',
	architecture_hint: 'llama'
};

describe('ModelStep', () => {
	it('renders the heading while scanning', () => {
		mocks.scan = { isPending: true, mutate: vi.fn() };
		render(ModelStep);
		expect(screen.getByRole('heading', { name: 'Models' })).toBeInTheDocument();
		expect(screen.queryByText('a.gguf')).toBeNull();
	});

	it('shows the model table + a truncation warning', () => {
		mocks.scan = {
			isPending: false,
			mutate: vi.fn(),
			data: { models: [model], truncated: true, truncation_reason: 'timeout', warnings: [] }
		};
		render(ModelStep);
		expect(screen.getByText('a.gguf')).toBeInTheDocument();
		expect(screen.getByText(/truncated/)).toBeInTheDocument();
	});

	it('shows an empty state when no models are found', () => {
		mocks.scan = {
			isPending: false,
			mutate: vi.fn(),
			data: { models: [], truncated: false, truncation_reason: null, warnings: [] }
		};
		render(ModelStep);
		expect(screen.getByText('No models found')).toBeInTheDocument();
	});
});
