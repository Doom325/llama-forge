import { describe, it, expect, vi, beforeEach } from 'vitest';
import { LiveDataStore } from './liveData.svelte';
import { QueryClient } from '@tanstack/svelte-query';

describe('LiveDataStore', () => {
	let queryClient: QueryClient;
	let store: LiveDataStore;
	let eventSourceMock: Record<string, unknown>;

	beforeEach(() => {
		queryClient = new QueryClient();
		queryClient.setQueryData = vi.fn();
		store = new LiveDataStore(queryClient);

		eventSourceMock = {
			close: vi.fn(),
			addEventListener: vi.fn()
		};
		const EventSourceSpy = vi.fn().mockImplementation(function (this: unknown) {
			return eventSourceMock;
		});
		vi.stubGlobal('EventSource', EventSourceSpy);
		vi.useFakeTimers();
	});

	it('manages subscriber lifecycle', () => {
		const unsub1 = store.subscribe();
		expect(store['eventSource']).toBeTruthy();

		const unsub2 = store.subscribe();
		expect(EventSource).toHaveBeenCalledTimes(1);

		unsub1();
		expect(eventSourceMock.close as import('vitest').Mock).not.toHaveBeenCalled();

		unsub2();
		expect(eventSourceMock.close as import('vitest').Mock).toHaveBeenCalledTimes(1);
		expect(store['eventSource']).toBeNull();
	});

	it('buffers and flushes events', () => {
		store.subscribe();

		// Simulate open
		(eventSourceMock.onopen as () => void)();

		// Get handler
		const serverUpdateCall = (
			eventSourceMock.addEventListener as import('vitest').Mock
		).mock.calls.find((c: unknown[]) => c[0] === 'server-update');
		const handler = serverUpdateCall?.[1] as (e: MessageEvent) => void;

		// Simulate message
		if (handler) {
			handler({
				type: 'server-update',
				data: JSON.stringify({ id: 'server1', status: 'healthy' })
			} as MessageEvent);
		}

		expect(queryClient.setQueryData).not.toHaveBeenCalled();

		// Advance RAF timer
		vi.advanceTimersByTime(150);
		// Force flush via visibility change since we can't easily advance RAF in JSDOM sometimes,
		// or call private flushBuffer
		store['flushBuffer']();

		expect(queryClient.setQueryData).toHaveBeenCalledWith(['server', 'server1'], {
			id: 'server1',
			status: 'healthy'
		});
	});
});
