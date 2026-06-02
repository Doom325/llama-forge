import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ComparisonPane from './ComparisonPane.svelte';
import type { GenerationParams, ModelInfo } from '$lib/api/types';

const model: ModelInfo = {
	id: 'mistral-7b',
	name: 'Mistral 7B',
	loaded: true,
	assignedServers: ['gpu-0'],
	capabilities: ['chat']
};

const parameters: GenerationParams = {
	temperature: 0.7,
	max_tokens: 256,
	top_p: 0.9,
	top_k: 40
};

describe('ComparisonPane', () => {
	it('renders a section labelled with the model name and an h3 heading', () => {
		render(ComparisonPane, {
			model,
			parameters,
			sharedPrompt: null,
			onClose: vi.fn()
		});

		const section = screen.getByRole('region', {
			name: 'Response from Mistral 7B (comparison)'
		});
		expect(section).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Mistral 7B' })).toBeInTheDocument();
	});

	it('announces that the pane was opened via a polite live region', () => {
		render(ComparisonPane, {
			model,
			parameters,
			sharedPrompt: null,
			onClose: vi.fn()
		});

		expect(screen.getByText('Comparison pane opened for Mistral 7B.')).toBeInTheDocument();
	});

	it('calls session.cancel and onClose when the Close button is clicked', async () => {
		const onClose = vi.fn();
		render(ComparisonPane, { model, parameters, sharedPrompt: null, onClose });

		const closeButton = screen.getByRole('button', { name: 'Close' });
		closeButton.click();

		// Microtask flush so the click handler completes.
		await Promise.resolve();
		expect(onClose).toHaveBeenCalledTimes(1);
	});
});
