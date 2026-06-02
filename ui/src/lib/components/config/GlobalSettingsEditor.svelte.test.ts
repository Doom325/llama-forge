import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import userEvent from '@testing-library/user-event';
import GlobalSettingsEditorWrapper from '$lib/components/testing/GlobalSettingsEditorWrapper.svelte';
import type { GlobalSettings } from '$lib/api/types';

// createConfigForm registers beforeNavigate; stub it.
vi.mock('$app/navigation', () => ({
	beforeNavigate: () => {}
}));

const validInitial: GlobalSettings = {
	bind_address: '0.0.0.0',
	grpc_port: 50051,
	http_port: 8080,
	log_level: 'info',
	log_format: 'pretty',
	metrics_retention_days: 7,
	queue_max_depth: 100,
	queue_default_timeout_secs: 60,
	restart_stagger_secs: 5
};

describe('GlobalSettingsEditor', () => {
	beforeEach(() => {
		vi.restoreAllMocks();
	});
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('renders an enabled Save button with a valid initial config', () => {
		render(GlobalSettingsEditorWrapper, {
			initial: validInitial,
			etag: 'v1',
			reload: vi.fn()
		});

		const save = screen.getByRole('button', { name: 'Save' });
		expect(save).toBeInTheDocument();
		expect(save).not.toBeDisabled();
	});

	it('shows an inline error when a port is out of range', async () => {
		const user = userEvent.setup();
		render(GlobalSettingsEditorWrapper, {
			initial: validInitial,
			etag: 'v1',
			reload: vi.fn()
		});

		const httpPort = screen.getByLabelText('HTTP port');
		await user.clear(httpPort);
		await user.type(httpPort, '70000');
		await user.tab(); // blur to mark touched

		expect(await screen.findByText('HTTP port must be between 1 and 65535.')).toBeInTheDocument();
		// Save is NOT disabled merely because validation failed.
		expect(screen.getByRole('button', { name: 'Save' })).not.toBeDisabled();
	});

	it('surfaces a conflict banner on 409 and reloads + rebases on click', async () => {
		const user = userEvent.setup();

		// First fetch (the PUT save) returns 409, triggering conflict.
		const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
			new Response(JSON.stringify({ message: 'conflict' }), {
				status: 409,
				headers: { 'Content-Type': 'application/json' }
			})
		);

		const fresh: GlobalSettings = { ...validInitial, http_port: 9090 };
		const reload = vi.fn().mockResolvedValue({ data: fresh, etag: 'v2' });

		render(GlobalSettingsEditorWrapper, {
			initial: validInitial,
			etag: 'v1',
			reload
		});

		// Make a change so submit proceeds, then save.
		const grpcPort = screen.getByLabelText('gRPC port');
		await user.clear(grpcPort);
		await user.type(grpcPort, '50052');
		await user.click(screen.getByRole('button', { name: 'Save' }));

		// Conflict banner appears.
		expect(
			await screen.findByText('Configuration was updated by another source.')
		).toBeInTheDocument();
		expect(fetchMock).toHaveBeenCalled();

		// Click the Reload action in the banner.
		const reloadBtn = screen.getByRole('button', { name: 'Reload' });
		await user.click(reloadBtn);

		expect(reload).toHaveBeenCalledTimes(1);

		// After rebase, the banner is gone and the fresh value is applied.
		await vi.waitFor(() => {
			expect(
				screen.queryByText('Configuration was updated by another source.')
			).not.toBeInTheDocument();
		});
		expect(screen.getByLabelText('HTTP port')).toHaveValue(9090);
	});
});
