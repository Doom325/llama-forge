import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi, beforeAll } from 'vitest';

const mocks = vi.hoisted(() => ({
	scan: { isPending: false, data: undefined, mutate: vi.fn() } as Record<string, unknown>
}));

vi.mock('$lib/stores/setup.svelte', () => ({
	useSetupStore: () => ({ sessionId: 's' })
}));
vi.mock('$lib/api/queries', () => ({
	createBinaryScanMutation: () => mocks.scan,
	createHardwareQuery: () => ({ data: undefined }),
	createBuildMutation: () => ({ mutateAsync: vi.fn() })
}));
vi.mock('$lib/api/queryClient', () => ({ queryClient: { setQueryData: vi.fn() } }));
vi.mock('$lib/api/client', () => ({ apiFetch: vi.fn() }));

// jsdom has no EventSource; the build stream effect doesn't run in these tests
// (no buildId), but stub it so the module never references an undefined global.
beforeAll(() => {
	class FakeEventSource {
		addEventListener() {}
		close() {}
		onerror: (() => void) | null = null;
	}
	(globalThis as unknown as { EventSource: unknown }).EventSource = FakeEventSource;
});

import BinaryStep from './+page.svelte';

describe('BinaryStep', () => {
	it('shows the found binary path + version', () => {
		mocks.scan = {
			isPending: false,
			mutate: vi.fn(),
			data: { found: true, path: '/usr/bin/llama-server', binary_version: 'b1', backends: [] }
		};
		render(BinaryStep);
		expect(screen.getByText('Found llama-server')).toBeInTheDocument();
		expect(screen.getByText('/usr/bin/llama-server')).toBeInTheDocument();
		expect(screen.getByText(/b1/)).toBeInTheDocument();
	});

	it('offers the build option when no binary is found', () => {
		mocks.scan = {
			isPending: false,
			mutate: vi.fn(),
			data: { found: false, searched_paths: [] }
		};
		render(BinaryStep);
		expect(screen.getByText('No llama-server found on this system.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Build llama-server' })).toBeInTheDocument();
	});
});
