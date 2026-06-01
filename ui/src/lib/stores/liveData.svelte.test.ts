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
		store['flushBuffer']();

		expect(queryClient.setQueryData).toHaveBeenCalledWith(['server', 'server1'], {
			id: 'server1',
			status: 'healthy'
		});
	});

	it('compacts correctly by retaining different event types for the same entity', () => {
		store.subscribe();
		(eventSourceMock.onopen as () => void)();

		const getHandler = (type: string) => {
			const call = (eventSourceMock.addEventListener as import('vitest').Mock).mock.calls.find(
				(c: unknown[]) => c[0] === type
			);
			return call?.[1] as (e: MessageEvent) => void;
		};

		const serverHandler = getHandler('server-update');
		const metricsHandler = getHandler('metrics-update');

		if (serverHandler && metricsHandler) {
			serverHandler({
				type: 'server-update',
				data: JSON.stringify({ id: 'server1', status: 'healthy' })
			} as MessageEvent);

			metricsHandler({
				type: 'metrics-update',
				data: JSON.stringify({ id: 'server1', throughput: 100 })
			} as MessageEvent);
		}

		store['flushBuffer']();

		expect(queryClient.setQueryData).toHaveBeenCalledWith(['server', 'server1'], {
			id: 'server1',
			status: 'healthy'
		});
		expect(queryClient.setQueryData).toHaveBeenCalledWith(['server', 'server1', 'metrics'], {
			id: 'server1',
			throughput: 100
		});
	});

	it('applies jitter to reconnect backoff', () => {
		const setTimeoutSpy = vi.spyOn(global, 'setTimeout');
		store.subscribe();

		// Mock Math.random to return 0.5 (middle of jitter range)
		const randomSpy = vi.spyOn(Math, 'random').mockReturnValue(0.5);

		// Trigger disconnect (first attempt)
		(eventSourceMock.onerror as () => void)();

		// Attempt 0: 1000 * 2^0 + 500 = 1500
		expect(setTimeoutSpy).toHaveBeenCalledWith(expect.any(Function), 1500);

		// Simulate timer firing to advance to reconnect attempt 1
		vi.advanceTimersByTime(1500);

		// Trigger disconnect again
		(eventSourceMock.onerror as () => void)();

		// Attempt 1: 1000 * 2^1 + 500 = 2500
		expect(setTimeoutSpy).toHaveBeenCalledWith(expect.any(Function), 2500);

		randomSpy.mockRestore();
		setTimeoutSpy.mockRestore();
	});

	it('maps event types to correct query keys', () => {
		expect(store['getQueryKey']('server-update', { id: 'server1' })).toEqual(['server', 'server1']);
		expect(store['getQueryKey']('metrics-update', { id: 'server2' })).toEqual([
			'server',
			'server2'
		]);
		expect(store['getQueryKey']('server-update', null)).toEqual(['servers']);
		expect(store['getQueryKey']('queue-update', null)).toEqual(['queue']);
		expect(store['getQueryKey']('config-changed', { section: 'servers' })).toEqual([
			'config',
			'servers'
		]);
		expect(store['getQueryKey']('model-update', null)).toEqual(['models']);
		expect(store['getQueryKey']('unknown-event', null)).toBeNull();
	});
});
