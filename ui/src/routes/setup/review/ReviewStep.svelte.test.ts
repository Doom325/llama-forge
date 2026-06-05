import { render, screen, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const mocks = vi.hoisted(() => ({
	preview: { isPending: false, data: { toml: '[global]\nbind = "127.0.0.1"' }, mutate: vi.fn() },
	saveMutateAsync: vi.fn(),
	validateMutateAsync: vi.fn(),
	fetchStatus: vi.fn(),
	goto: vi.fn()
}));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$lib/utils/markdown', () => ({ renderMarkdown: (s: string) => s }));
vi.mock('$lib/stores/setup.svelte', () => ({
	useSetupStore: () => ({
		sessionId: 's',
		selectedModelIds: new Set(['m1']),
		userOverrides: new Map(),
		savePath: ''
	})
}));
vi.mock('$lib/api/queries', () => ({
	createPreviewConfigMutation: () => mocks.preview,
	createSetupSaveConfigMutation: () => ({ isPending: false, mutateAsync: mocks.saveMutateAsync }),
	createValidateLaunchMutation: () => ({
		isPending: false,
		mutateAsync: mocks.validateMutateAsync
	}),
	fetchSetupStatus: () => mocks.fetchStatus()
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
								size_bytes: 1,
								quantization_hint: '',
								architecture_hint: ''
							}
						]
					}
				: key[1] === 'scan-binary'
					? { found: true, path: '/usr/bin/llama-server' }
					: key[1] === 'recommend'
						? {
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
						: undefined
	}
}));

beforeEach(() => {
	mocks.saveMutateAsync = vi.fn().mockResolvedValue({ path: '/x', replaced_existing: false });
	mocks.validateMutateAsync = vi.fn().mockResolvedValue({ success: true, message: 'ok' });
	mocks.fetchStatus = vi.fn().mockResolvedValue({ setup_required: false, version: '0' });
});
afterEach(() => vi.useRealTimers());

import ReviewStep from './+page.svelte';

describe('ReviewStep', () => {
	it('renders the generated TOML preview', () => {
		render(ReviewStep);
		expect(screen.getByText(/\[global\]/)).toBeInTheDocument();
	});

	it('save → validate → status poll → success shows Go to Dashboard', async () => {
		vi.useFakeTimers();
		render(ReviewStep);
		await fireEvent.click(screen.getByRole('button', { name: 'Save & Launch' }));
		// flush save + validate mutateAsync
		await vi.advanceTimersByTimeAsync(0);
		await tick();
		// advance through the 2s poll
		await vi.advanceTimersByTimeAsync(2000);
		await tick();
		expect(mocks.saveMutateAsync).toHaveBeenCalledOnce();
		expect(mocks.validateMutateAsync).toHaveBeenCalledOnce();
		expect(screen.getByRole('button', { name: 'Go to Dashboard' })).toBeInTheDocument();
	});

	it('times out after 5 failed polls', async () => {
		mocks.fetchStatus = vi.fn().mockResolvedValue({ setup_required: true, version: '0' });
		vi.useFakeTimers();
		render(ReviewStep);
		await fireEvent.click(screen.getByRole('button', { name: 'Save & Launch' }));
		await vi.advanceTimersByTimeAsync(0);
		await tick();
		// 5 polls × 2s
		await vi.advanceTimersByTimeAsync(10_000);
		await tick();
		expect(screen.getByRole('button', { name: 'Go to Dashboard anyway' })).toBeInTheDocument();
	});
});
