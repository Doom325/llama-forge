import { SSE_ENDPOINT } from '$lib/utils/constants';
import type { QueryClient } from '@tanstack/svelte-query';
import type { ServerState } from '$lib/api/types';

export class LiveDataStore {
	connected = $state(false);
	private subscriberCount = 0;
	private eventSource: EventSource | null = null;
	private queryClient: QueryClient;

	private reconnectAttempt = 0;
	private reconnectTimeoutId: ReturnType<typeof setTimeout> | null = null;
	private lastEventId = '';

	private heartbeatTimeoutId: ReturnType<typeof setTimeout> | null = null;

	// eslint-disable-next-line svelte/prefer-svelte-reactivity
	private buffer: Map<string, { type: string; data: unknown }> = new Map();
	private rafId: number | null = null;
	private lastFlushTime = 0;

	constructor(queryClient: QueryClient) {
		this.queryClient = queryClient;

		if (typeof document !== 'undefined') {
			document.addEventListener('visibilitychange', this.handleVisibilityChange.bind(this));
		}
	}

	subscribe() {
		this.subscriberCount++;
		if (this.subscriberCount === 1) {
			this.connect();
		}
		return () => this.unsubscribe();
	}

	unsubscribe() {
		this.subscriberCount = Math.max(0, this.subscriberCount - 1);
		if (this.subscriberCount === 0) {
			this.disconnect();
		}
	}

	private connect() {
		if (this.eventSource) return;
		if (this.reconnectTimeoutId) {
			clearTimeout(this.reconnectTimeoutId);
			this.reconnectTimeoutId = null;
		}

		let url = SSE_ENDPOINT;
		if (this.lastEventId) {
			url += `?last_event_id=${encodeURIComponent(this.lastEventId)}`;
		}

		this.eventSource = new EventSource(url);

		this.eventSource.onopen = () => {
			this.connected = true;
			this.reconnectAttempt = 0;
			this.resetHeartbeat();
		};

		this.eventSource.onerror = () => {
			this.handleDisconnect();
		};

		const handleMessage = (e: MessageEvent) => {
			if (e.lastEventId) {
				this.lastEventId = e.lastEventId;
			}
			this.resetHeartbeat();

			try {
				const data = JSON.parse(e.data);
				this.bufferEvent(e.type, data);
			} catch {
				// Parse error
			}
		};

		this.eventSource.addEventListener('server-update', handleMessage);
		this.eventSource.addEventListener('metrics-update', handleMessage);
		this.eventSource.addEventListener('queue-update', handleMessage);
		this.eventSource.addEventListener('config-changed', handleMessage);
		this.eventSource.addEventListener('model-update', handleMessage);
	}

	private disconnect() {
		if (this.eventSource) {
			this.eventSource.close();
			this.eventSource = null;
		}
		if (this.heartbeatTimeoutId) {
			clearTimeout(this.heartbeatTimeoutId);
			this.heartbeatTimeoutId = null;
		}
		if (this.reconnectTimeoutId) {
			clearTimeout(this.reconnectTimeoutId);
			this.reconnectTimeoutId = null;
		}
		if (this.rafId) {
			cancelAnimationFrame(this.rafId);
			this.rafId = null;
		}
		this.connected = false;
	}

	private handleDisconnect() {
		if (this.eventSource) {
			this.eventSource.close();
			this.eventSource = null;
		}
		this.connected = false;

		if (this.subscriberCount > 0 && !this.reconnectTimeoutId) {
			const delay = Math.min(
				1000 * Math.pow(2, this.reconnectAttempt) + Math.random() * 1000,
				30_000
			);
			this.reconnectAttempt++;
			this.reconnectTimeoutId = setTimeout(() => this.connect(), delay);
		}
	}

	private resetHeartbeat() {
		if (this.heartbeatTimeoutId) {
			clearTimeout(this.heartbeatTimeoutId);
		}
		// 30 seconds = 2 missed 15s heartbeats
		this.heartbeatTimeoutId = setTimeout(() => {
			this.handleDisconnect();
		}, 30_000);
	}

	private getQueryKey(type: string, data: unknown): string[] | null {
		const typedData = data as Record<string, unknown>;
		switch (type) {
			case 'server-update':
			case 'metrics-update':
				if (typedData && typedData.id) return ['server', String(typedData.id)];
				return ['servers']; // fallback if bulk
			case 'queue-update':
				return ['queue'];
			case 'config-changed':
				return ['config', String(typedData.section)];
			case 'model-update':
				return ['models'];
			default:
				return null;
		}
	}

	private bufferEvent(type: string, data: unknown) {
		const keyArray = this.getQueryKey(type, data);
		if (!keyArray) return;
		const bufferKey = JSON.stringify(keyArray);

		this.buffer.set(bufferKey, { type, data });

		if (typeof document !== 'undefined' && document.hidden) {
			// Compaction mode, wait for focus
			return;
		}

		this.scheduleFlush();
	}

	private scheduleFlush() {
		if (this.rafId) return; // already scheduled

		this.rafId = requestAnimationFrame((timestamp) => {
			this.rafId = null;
			if (timestamp - this.lastFlushTime < 100) {
				// Too soon, schedule next frame
				this.scheduleFlush();
				return;
			}

			this.flushBuffer();
			this.lastFlushTime = timestamp;
		});
	}

	private flushBuffer() {
		if (this.buffer.size === 0) return;

		const entries = Array.from(this.buffer.entries());
		this.buffer.clear();

		for (const [keyStr, event] of entries) {
			const queryKey = JSON.parse(keyStr);

			if (event.type === 'server-update') {
				this.queryClient.setQueryData(queryKey, event.data);
				// Also update the global list if present
				this.queryClient.setQueryData(['servers'], (old: ServerState[] | undefined) => {
					if (!old) return old;
					return old.map((s) =>
						s.id === (event.data as ServerState).id ? (event.data as ServerState) : s
					);
				});
			} else if (event.type === 'metrics-update') {
				this.queryClient.setQueryData([...queryKey, 'metrics'], event.data);
			} else if (event.type === 'queue-update') {
				this.queryClient.setQueryData(['queue'], event.data);
			} else if (event.type === 'config-changed') {
				this.queryClient.setQueryData(
					['config', (event.data as Record<string, unknown>).section],
					(event.data as Record<string, unknown>).config
				);
			} else if (event.type === 'model-update') {
				this.queryClient.setQueryData(['models'], event.data);
			}
		}
	}

	private handleVisibilityChange() {
		if (typeof document !== 'undefined' && !document.hidden) {
			this.flushBuffer();
		}
	}
}
