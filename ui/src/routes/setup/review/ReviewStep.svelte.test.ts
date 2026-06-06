import { render, screen, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const mocks = vi.hoisted(() => ({
	preview: { isPending: false, data: { toml: '[global]\nbind = "127.0.0.1"' }, mutate: vi.fn() },
	saveMutateAsync: vi.fn(),
	validateMutateAsync: vi.fn(),
	fetchStatus: vi.fn(),
	goto: vi.fn(),
	reconfigure: false
}));

vi.mock('$app/navigation', () => ({ goto: mocks.goto }));
vi.mock('$lib/utils/markdown', () => ({ renderMarkdown: (s: string) => s }));
vi.mock('$lib/stores/setup.svelte', () => ({
	useSetupStore: () => ({
		sessionId: 's',
		selectedModelIds: new Set(['m1']),
		userOverrides: new Map(),
		savePath: '',
		reconfigure: mocks.reconfigure
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
	mocks.reconfigure = false;
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

	// 13.19 — standalone save fails, the error is shown and the user stays on the
	// review step, then a retry succeeds through to completion.
	it('save failure → retry → success', async () => {
		mocks.saveMutateAsync = vi
			.fn()
			.mockRejectedValueOnce(new Error('permission denied'))
			.mockResolvedValue({ path: '/x', replaced_existing: false });
		vi.useFakeTimers();
		render(ReviewStep);

		await fireEvent.click(screen.getByRole('button', { name: 'Save & Launch' }));
		await vi.advanceTimersByTimeAsync(0);
		await tick();
		// Error surfaced; still on the review step (Save & Launch present, no poll).
		expect(screen.getByText(/permission denied/)).toBeInTheDocument();
		expect(mocks.validateMutateAsync).not.toHaveBeenCalled();

		await fireEvent.click(screen.getByRole('button', { name: 'Save & Launch' }));
		await vi.advanceTimersByTimeAsync(0);
		await tick();
		await vi.advanceTimersByTimeAsync(2000);
		await tick();
		expect(screen.getByRole('button', { name: 'Go to Dashboard' })).toBeInTheDocument();
		vi.useRealTimers();
	});

	// #274 — in the reconfigure flow the config already exists, so the save must
	// force-overwrite (the backend merges) rather than 409.
	it('reconfigure save forces overwrite', async () => {
		mocks.reconfigure = true;
		mocks.saveMutateAsync = vi.fn().mockResolvedValue({ path: '/x', replaced_existing: true });
		vi.useFakeTimers();
		render(ReviewStep);

		await fireEvent.click(screen.getByRole('button', { name: 'Save & Launch' }));
		await vi.advanceTimersByTimeAsync(0);
		await tick();

		expect(mocks.saveMutateAsync).toHaveBeenCalledWith(expect.objectContaining({ force: true }));
		vi.useRealTimers();
	});
});
