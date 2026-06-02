import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import ModelCard from './ModelCard.svelte';
import type { ModelInfo } from '$lib/api/types';

const baseModel: ModelInfo = {
	id: 'llama-3-8b',
	name: 'Llama 3 8B',
	loaded: true,
	assignedServers: ['gpu-0', 'gpu-1'],
	capabilities: ['chat', 'completion']
};

describe('ModelCard', () => {
	it('renders the model name and id', () => {
		render(ModelCard, { model: baseModel });
		expect(screen.getByRole('heading', { name: 'Llama 3 8B' })).toBeInTheDocument();
		expect(screen.getByText('ID: llama-3-8b')).toBeInTheDocument();
	});

	it('shows a Loaded badge when loaded is true', () => {
		render(ModelCard, { model: { ...baseModel, loaded: true } });
		expect(screen.getByText('Loaded')).toBeInTheDocument();
		expect(screen.queryByText('Not loaded')).not.toBeInTheDocument();
	});

	it('shows a Not loaded badge when loaded is false', () => {
		render(ModelCard, { model: { ...baseModel, loaded: false } });
		expect(screen.getByText('Not loaded')).toBeInTheDocument();
		expect(screen.queryByText('Loaded')).not.toBeInTheDocument();
	});

	it('renders assigned servers and capabilities', () => {
		render(ModelCard, { model: baseModel });
		expect(screen.getByText('gpu-0')).toBeInTheDocument();
		expect(screen.getByText('gpu-1')).toBeInTheDocument();
		expect(screen.getByText('chat')).toBeInTheDocument();
		expect(screen.getByText('completion')).toBeInTheDocument();
	});

	it('handles empty servers and capabilities gracefully', () => {
		render(ModelCard, {
			model: { ...baseModel, assignedServers: [], capabilities: [] }
		});
		// Two "None" placeholders: one for servers, one for capabilities.
		expect(screen.getAllByText('None')).toHaveLength(2);
	});
});
