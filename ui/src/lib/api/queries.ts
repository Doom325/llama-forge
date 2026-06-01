import { createQuery, createMutation } from '@tanstack/svelte-query';
import { apiFetch } from './client';
import { queryClient } from './queryClient';
import type {
	ServerState,
	ServerMetricsSummary,
	QueueItem,
	RequestRecord,
	ModelInfo,
	ConfigSection
} from './types';

// Queries
export function createServersQuery() {
	return createQuery(() => ({
		queryKey: ['servers'],
		queryFn: () => apiFetch<ServerState[]>('/api/servers')
	}));
}

export function createServerDetailQuery(id: string) {
	return createQuery(() => ({
		queryKey: ['server', id],
		queryFn: () => apiFetch<ServerState>(`/api/servers/${id}`)
	}));
}

export function createServerMetricsQuery(id: string) {
	return createQuery(() => ({
		queryKey: ['server', id, 'metrics'],
		queryFn: () => apiFetch<ServerMetricsSummary>(`/api/servers/${id}/metrics`)
	}));
}

export function createQueueQuery() {
	return createQuery(() => ({
		queryKey: ['queue'],
		queryFn: () => apiFetch<QueueItem[]>('/api/queue')
	}));
}

export function createRequestsQuery(filters: Record<string, string>) {
	return createQuery(() => ({
		queryKey: ['requests', filters],
		queryFn: () => {
			const searchParams = new URLSearchParams(filters);
			return apiFetch<RequestRecord[]>(`/api/requests?${searchParams.toString()}`);
		}
	}));
}

export function createAggregatedMetricsQuery(filters: Record<string, string>) {
	return createQuery(() => ({
		queryKey: ['metrics', 'aggregated', filters],
		queryFn: () => {
			const searchParams = new URLSearchParams(filters);
			return apiFetch<unknown>(`/api/metrics?${searchParams.toString()}`);
		}
	}));
}

export function createConfigQuery(section: ConfigSection) {
	return createQuery(() => ({
		queryKey: ['config', section],
		queryFn: () => apiFetch<unknown>(`/api/config/${section}`)
	}));
}

export function createModelsQuery() {
	return createQuery(() => ({
		queryKey: ['models'],
		queryFn: () => apiFetch<ModelInfo[]>('/api/models')
	}));
}

// Mutations
export function createRestartServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/restart`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		}
	}));
}

export function createStopServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/stop`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		}
	}));
}

export function createStartServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/start`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		}
	}));
}

export function createForceKillServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/force_kill`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		}
	}));
}

export function createCancelRequestMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/requests/${id}/cancel`, { method: 'POST' }),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ['queue'] });
		}
	}));
}

export function createSaveConfigMutation(section: ConfigSection) {
	return createMutation(() => ({
		mutationFn: ({ data, etag }: { data: unknown; etag: string }) =>
			apiFetch(`/api/config/${section}`, {
				method: 'PUT',
				body: JSON.stringify(data),
				headers: { 'If-Match': etag }
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ['config', section] });
		}
	}));
}

// Prefetching helpers
export function prefetchServerDetail(id: string) {
	queryClient.prefetchQuery({
		queryKey: ['server', id],
		queryFn: () => apiFetch<ServerState>(`/api/servers/${id}`)
	});
}

export function prefetchRouteData(routeKey: string) {
	if (routeKey === 'dashboard') {
		queryClient.prefetchQuery({
			queryKey: ['servers'],
			queryFn: () => apiFetch<ServerState[]>('/api/servers')
		});
	} else if (routeKey === 'queue') {
		queryClient.prefetchQuery({
			queryKey: ['queue'],
			queryFn: () => apiFetch<QueueItem[]>('/api/queue')
		});
	} else if (routeKey === 'models') {
		queryClient.prefetchQuery({
			queryKey: ['models'],
			queryFn: () => apiFetch<ModelInfo[]>('/api/models')
		});
	}
}

export function prefetchExplorerNextPage(filters: Record<string, string>, currentPage: number) {
	const nextFilters = { ...filters, page: String(currentPage + 1) };
	queryClient.prefetchQuery({
		queryKey: ['requests', nextFilters],
		queryFn: () => {
			const searchParams = new URLSearchParams(nextFilters);
			return apiFetch<RequestRecord[]>(`/api/requests?${searchParams.toString()}`);
		}
	});
}
