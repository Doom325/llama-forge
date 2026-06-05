import { render, screen, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const mocks = vi.hoisted(() => ({
	scan: { isPending: false, data: undefined, mutate: vi.fn() } as Record<string, unknown>,
	buildMutateAsync: vi.fn()
}));

vi.mock('$lib/stores/setup.svelte', () => ({
	useSetupStore: () => ({ sessionId: 's' })
}));
vi.mock('$lib/api/queries', () => ({
	createBinaryScanMutation: () => mocks.scan,
	createHardwareQuery: () => ({ data: undefined }),
	createBuildMutation: () => ({ mutateAsync: mocks.buildMutateAsync })
}));
vi.mock('$lib/api/queryClient', () => ({ queryClient: { setQueryData: vi.fn() } }));
vi.mock('$lib/api/client', () => ({ apiFetch: vi.fn() }));

// Controllable EventSource so tests can dispatch named events + simulate a drop.
class MockEventSource {
	static instances: MockEventSource[] = [];
	listeners: Record<string, (e: { data: string }) => void> = {};
	onerror: (() => void) | null = null;
	constructor(public url: string) {
		MockEventSource.instances.push(this);
	}
	addEventListener(name: string, fn: (e: { data: string }) => void) {
		this.listeners[name] = fn;
	}
	close() {}
	emit(name: string, data: unknown) {
		this.listeners[name]?.({ data: JSON.stringify(data) });
	}
	fail() {
		this.onerror?.();
	}
}

beforeEach(() => {
	MockEventSource.instances = [];
	mocks.buildMutateAsync = vi.fn().mockResolvedValue({ build_id: 'b1' });
	vi.stubGlobal('EventSource', MockEventSource);
});
afterEach(() => {
	vi.unstubAllGlobals();
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
		mocks.scan = { isPending: false, mutate: vi.fn(), data: { found: false, searched_paths: [] } };
		render(BinaryStep);
		expect(screen.getByText('No llama-server found on this system.')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Build llama-server' })).toBeInTheDocument();
	});

	it('does not duplicate log lines when the build stream reconnects', async () => {
		vi.useFakeTimers();
		mocks.scan = { isPending: false, mutate: vi.fn(), data: { found: false, searched_paths: [] } };
		render(BinaryStep);

		// Start a build (the backend replays the full backlog on each connection).
		await fireEvent.input(screen.getByPlaceholderText('/usr/local/bin'), {
			target: { value: '/opt' }
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Build llama-server' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Start build' }));
		// Flush mutateAsync + the $effect that opens the EventSource.
		await tick();
		await Promise.resolve();
		await tick();

		const es1 = MockEventSource.instances.at(-1);
		expect(es1).toBeDefined();
		es1!.emit('phase', { phase: 'building', message: '' });
		es1!.emit('log', { lines: ['LINE-A'] });
		await tick();

		const occurrences = () => (screen.getByRole('log').textContent?.match(/LINE-A/g) ?? []).length;
		expect(occurrences()).toBe(1);

		// Drop the connection → reconnect after the backoff delay.
		es1!.fail();
		await vi.advanceTimersByTimeAsync(1000);
		const es2 = MockEventSource.instances.at(-1);
		expect(es2).toBeDefined();
		expect(es2).not.toBe(es1);

		// The backend replays the FULL backlog on the new connection.
		es2!.emit('phase', { phase: 'building', message: '' });
		es2!.emit('log', { lines: ['LINE-A'] });
		await tick();

		// State was rebuilt, so the line appears exactly once — not doubled.
		expect(occurrences()).toBe(1);
		vi.useRealTimers();
	});
});
