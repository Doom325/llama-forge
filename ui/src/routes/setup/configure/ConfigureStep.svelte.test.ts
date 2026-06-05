import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
	rec: { isPending: false, mutate: vi.fn(), data: undefined } as Record<string, unknown>,
	userOverrides: new Map<string, Record<string, unknown>>()
}));

vi.mock('$lib/stores/setup.svelte', () => ({
	useSetupStore: () => ({
		sessionId: 's',
		selectedModelIds: new Set(['m1']),
		userOverrides: mocks.userOverrides
	})
}));
vi.mock('$lib/api/queries', () => ({
	createRecommendationMutation: () => mocks.rec
}));
vi.mock('$lib/api/queryClient', () => ({
	queryClient: {
		getQueryData: (key: string[]) =>
			key[1] === 'scan-models'
				? {
						models: [
							{
								id: 'm1',
								path: '/a.gguf',
								filename: 'a.gguf',
								size_bytes: 1024,
								quantization_hint: 'Q4',
								architecture_hint: 'llama'
							}
						]
					}
				: key[1] === 'scan-binary'
					? { found: true, path: '/usr/bin/llama-server' }
					: undefined
	}
}));

import ConfigureStep from './+page.svelte';

const RECOMMENDATION = {
	isPending: false,
	mutate: vi.fn(),
	data: {
		servers: [
			{
				model_path: '/a.gguf',
				recommended: {
					gpu_layers: 33,
					ctx_size: 4096,
					parallel_slots: 4,
					cache_type_k: 'f16',
					cache_type_v: 'f16',
					flash_attn: 'auto',
					estimated_vram_mb: 8000,
					port: 8001
				},
				rationale: {}
			}
		],
		warnings: []
	}
};

describe('ConfigureStep', () => {
	it('renders a config card per selected model with recommended values', () => {
		mocks.rec = RECOMMENDATION;
		mocks.userOverrides = new Map();
		render(ConfigureStep);
		expect(screen.getByText('a.gguf')).toBeInTheDocument();
		expect((screen.getByLabelText('Port') as HTMLInputElement).value).toBe('8001');
		// alias defaults to the filename without extension
		expect((screen.getByLabelText('Alias') as HTMLInputElement).value).toBe('a');
	});

	it('merges the user override over the recommendation at render', () => {
		mocks.rec = RECOMMENDATION;
		mocks.userOverrides = new Map([['m1', { alias: 'custom-alias', port: 9999 }]]);
		render(ConfigureStep);
		expect((screen.getByLabelText('Alias') as HTMLInputElement).value).toBe('custom-alias');
		expect((screen.getByLabelText('Port') as HTMLInputElement).value).toBe('9999');
	});
});
