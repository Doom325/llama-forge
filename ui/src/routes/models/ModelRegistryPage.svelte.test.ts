import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi, afterEach } from 'vitest';
import ModelRegistryPageWrapper from '$lib/components/testing/ModelRegistryPageWrapper.svelte';
import * as queries from '$lib/api/queries';
import type { ModelInfo } from '$lib/api/types';

type ModelsQuery = ReturnType<typeof queries.createModelsQuery>;

function mockModelsQuery(value: Partial<ModelsQuery>) {
	vi.spyOn(queries, 'createModelsQuery').mockReturnValue(value as unknown as ModelsQuery);
}

const sampleModels: ModelInfo[] = [
	{ id: 'a', name: 'Model A', loaded: true, assignedServers: ['gpu-0'], capabilities: ['chat'] },
	{ id: 'b', name: 'Model B', loaded: false, assignedServers: [], capabilities: [] },
	{ id: 'c', name: 'Model C', loaded: true, assignedServers: ['gpu-1'], capabilities: ['embed'] }
];

describe('ModelRegistryPage', () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('renders a card per model when data is present', () => {
		mockModelsQuery({ isPending: false, isError: false, data: sampleModels });
		render(ModelRegistryPageWrapper);

		expect(screen.getByRole('heading', { level: 1, name: 'Models' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Model A' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Model B' })).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Model C' })).toBeInTheDocument();
	});

	it('shows the empty state when there are no models', () => {
		mockModelsQuery({ isPending: false, isError: false, data: [] });
		render(ModelRegistryPageWrapper);

		expect(screen.getByText('No models')).toBeInTheDocument();
		expect(screen.queryByRole('heading', { name: 'Model A' })).not.toBeInTheDocument();
	});

	it('shows skeletons while pending', () => {
		mockModelsQuery({ isPending: true, isError: false, data: undefined });
		render(ModelRegistryPageWrapper);

		expect(screen.queryByText('No models')).not.toBeInTheDocument();
		expect(screen.queryByRole('heading', { name: 'Model A' })).not.toBeInTheDocument();
	});

	it('shows an error state with a retry action', () => {
		const refetch = vi.fn();
		mockModelsQuery({ isPending: false, isError: true, refetch });
		render(ModelRegistryPageWrapper);

		const retry = screen.getByRole('button', { name: 'Retry' });
		expect(retry).toBeInTheDocument();
		retry.click();
		expect(refetch).toHaveBeenCalledTimes(1);
	});
});
