import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import BuildProgress from './BuildProgress.svelte';
import type { BuildStatus } from '$lib/api/types';

describe('BuildProgress', () => {
	it('running shows a progress bar, log, and cancel button', () => {
		const status: BuildStatus = {
			status: 'running',
			build_id: 'b',
			phase: 'building',
			percent: 40,
			logLines: ['compiling object']
		};
		render(BuildProgress, { status });
		expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '40');
		expect(screen.getByRole('log')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: /Cancel/ })).toBeInTheDocument();
	});

	it('failed shows the error + retry/skip/change-backend and fires onretry', async () => {
		const onretry = vi.fn();
		const status: BuildStatus = {
			status: 'failed',
			build_id: 'b',
			phase: 'building',
			error: 'cmake exploded',
			lastLines: ['err']
		};
		render(BuildProgress, { status, onretry });
		expect(screen.getByText(/cmake exploded/)).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Skip' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Change backend' })).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
		expect(onretry).toHaveBeenCalledOnce();
	});

	it('complete shows success + the installed binary path', () => {
		const status: BuildStatus = {
			status: 'complete',
			build_id: 'b',
			binary_path: '/x/llama-server',
			duration_secs: 5,
			replaced_existing: false,
			backup_path: null
		};
		render(BuildProgress, { status });
		expect(screen.getByText('Build complete')).toBeInTheDocument();
		expect(screen.getByText('/x/llama-server')).toBeInTheDocument();
	});
});
