import { createQuery, createMutation } from '@tanstack/svelte-query';
import { apiFetch, apiFetchWithMeta } from './client';
import { queryClient, liveDataStore } from './queryClient';
import { toastStore } from '$lib/stores/toast.svelte';
import type {
	ServerState,
	ServerMetricsSummary,
	QueueItem,
	RequestRecord,
	ModelInfo,
	ConfigSection,
	HardwareInfo,
	BinaryScanResult,
	ModelScanRequest,
	ModelScanResult,
	RecommendationRequest,
	RecommendationResult,
	SaveConfigRequest,
	ServerConfigDraft,
	SetupBackend
} from './types';

// Queries
export function createServersQuery() {
	return createQuery(() => ({
		queryKey: ['servers'],
		queryFn: () => apiFetch<ServerState[]>('/api/servers'),
		refetchInterval: () => (liveDataStore.connected ? false : 5000)
	}));
}

export function createServerDetailQuery(id: string) {
	return createQuery(() => ({
		queryKey: ['server', id],
		queryFn: () => apiFetch<ServerState>(`/api/servers/${id}`),
		refetchInterval: () => (liveDataStore.connected ? false : 5000)
	}));
}

export function createServerMetricsQuery(id: string) {
	return createQuery(() => ({
		queryKey: ['server', id, 'metrics'],
		queryFn: () => apiFetch<ServerMetricsSummary>(`/api/servers/${id}/metrics`),
		refetchInterval: () => (liveDataStore.connected ? false : 5000)
	}));
}

export function createQueueQuery() {
	return createQuery(() => ({
		queryKey: ['queue'],
		queryFn: () => apiFetch<QueueItem[]>('/api/queue'),
		staleTime: 2000,
		gcTime: 300_000, // 5 min
		refetchInterval: () => (liveDataStore.connected ? false : 5000)
	}));
}

export function createRequestsQuery(getFilters: () => Record<string, string>) {
	return createQuery(() => ({
		queryKey: ['requests', getFilters()],
		queryFn: () => {
			const searchParams = new URLSearchParams(getFilters());
			return apiFetch<RequestRecord[]>(`/api/requests?${searchParams.toString()}`);
		},
		staleTime: 30_000,
		gcTime: 300_000 // 5 min
	}));
}

export function createAggregatedMetricsQuery(filters: Record<string, string>) {
	return createQuery(() => ({
		queryKey: ['metrics', 'aggregated', filters],
		queryFn: () => {
			const searchParams = new URLSearchParams(filters);
			return apiFetch<import('./types').AggregatedMetrics>(
				`/api/metrics?${searchParams.toString()}`
			);
		},
		staleTime: 30_000,
		gcTime: 300_000 // 5 min
	}));
}

// Returns `{ data, etag }` — the ETag is captured at load time so the editor
// can send it back as `If-Match` on save for optimistic concurrency control.
export function createConfigQuery<T = unknown>(section: ConfigSection) {
	return createQuery(() => ({
		queryKey: ['config', section],
		queryFn: () => apiFetchWithMeta<T>(`/api/config/${section}`),
		staleTime: 60_000,
		gcTime: 600_000 // 10 min
	}));
}

export function createModelsQuery() {
	return createQuery(() => ({
		queryKey: ['models'],
		queryFn: () => apiFetch<ModelInfo[]>('/api/models'),
		staleTime: 60_000,
		gcTime: 600_000 // 10 min
	}));
}

// Mutations
export function createRestartServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/restart`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			toastStore.add('success', 'Server restart initiated');
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		},
		onError: (error: Error) => {
			toastStore.add('error', `Failed to restart server: ${error.message}`);
		}
	}));
}

export function createStopServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/stop`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			toastStore.add('success', 'Server stop initiated');
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		},
		onError: (error: Error) => {
			toastStore.add('error', `Failed to stop server: ${error.message}`);
		}
	}));
}

export function createStartServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/start`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			toastStore.add('success', 'Server start initiated');
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		},
		onError: (error: Error) => {
			toastStore.add('error', `Failed to start server: ${error.message}`);
		}
	}));
}

export function createForceKillServerMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/servers/${id}/force_kill`, { method: 'POST' }),
		onSuccess: (_data: unknown, id: string) => {
			toastStore.add('success', 'Server killed');
			queryClient.invalidateQueries({ queryKey: ['server', id] });
			queryClient.invalidateQueries({ queryKey: ['servers'] });
		},
		onError: (error: Error) => {
			toastStore.add('error', `Failed to kill server: ${error.message}`);
		}
	}));
}

export function createCancelRequestMutation() {
	return createMutation(() => ({
		mutationFn: (id: string) => apiFetch(`/api/requests/${id}/cancel`, { method: 'POST' }),
		onSuccess: () => {
			toastStore.add('success', 'Request cancelled');
			queryClient.invalidateQueries({ queryKey: ['queue'] });
		},
		onError: (error: Error) => {
			toastStore.add('error', `Failed to cancel request: ${error.message}`);
		}
	}));
}

export function createSaveConfigMutation(section: ConfigSection) {
	return createMutation(() => ({
		// Returns { data, etag }: the PUT response echoes the new ETag so the form
		// can refresh its If-Match version after a successful save.
		mutationFn: ({ data, etag }: { data: unknown; etag: string }) =>
			apiFetchWithMeta<unknown>(`/api/config/${section}`, {
				method: 'PUT',
				body: JSON.stringify(data),
				headers: { 'If-Match': etag }
			}),
		onSuccess: () => {
			toastStore.add('success', 'Configuration saved successfully');
			queryClient.invalidateQueries({ queryKey: ['config', section] });
		},
		onError: (error: Error) => {
			toastStore.add('error', `Failed to save configuration: ${error.message}`);
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
	} else if (routeKey === 'config/servers') {
		queryClient.prefetchQuery({
			queryKey: ['config', 'servers'],
			queryFn: () => apiFetch<unknown>('/api/config/servers')
		});
	} else if (routeKey === 'config/priority') {
		queryClient.prefetchQuery({
			queryKey: ['config', 'priority'],
			queryFn: () => apiFetch<unknown>('/api/config/priority')
		});
	} else if (routeKey === 'explorer') {
		queryClient.prefetchQuery({
			queryKey: ['requests', { page: '1' }],
			queryFn: () => {
				const searchParams = new URLSearchParams({ page: '1' });
				return apiFetch<RequestRecord[]>(`/api/requests?${searchParams.toString()}`);
			}
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

// ---------------------------------------------------------------------------
// Setup wizard (08_SETUP_WIZARD.md). The session id is attached as the
// `X-Setup-Session-Id` header on every call. /api/setup/* uses a FLAT version
// envelope, so each query types the inner payload shape directly. Scan/recommend
// are mutations (POST) whose results are written into the query cache so the
// step state machine's `getQueryData` checks can read them.
// ---------------------------------------------------------------------------

export function setupHeaders(sessionId: string, reconfigure: boolean): Record<string, string> {
	const headers: Record<string, string> = { 'X-Setup-Session-Id': sessionId };
	// In running mode the setup endpoints are gated behind this header (§13.8);
	// the wizard sends it for the reconfigure flow (Flow 4).
	if (reconfigure) {
		headers['X-Setup-Reconfigure'] = 'true';
	}
	return headers;
}

export function createHardwareQuery(
	sessionId: string,
	reconfigure: boolean,
	enabled: () => boolean = () => true
) {
	return createQuery(() => ({
		queryKey: ['setup', 'hardware'],
		queryFn: () =>
			apiFetch<HardwareInfo>('/api/setup/hardware', {
				headers: setupHeaders(sessionId, reconfigure)
			}),
		enabled: enabled(),
		staleTime: Infinity,
		retry: false
	}));
}

export function createBinaryScanMutation(sessionId: string, reconfigure: boolean) {
	return createMutation(() => ({
		// `BinaryScanRequest.extra_paths` — extra dirs searched ahead of $PATH.
		mutationFn: (extraPaths: string[]) =>
			apiFetch<BinaryScanResult>('/api/setup/scan-binary', {
				method: 'POST',
				body: JSON.stringify({ extra_paths: extraPaths }),
				headers: setupHeaders(sessionId, reconfigure)
			}),
		onSuccess: (data: BinaryScanResult) => queryClient.setQueryData(['setup', 'scan-binary'], data)
	}));
}

export function createModelScanMutation(sessionId: string, reconfigure: boolean) {
	return createMutation(() => ({
		mutationFn: (request: ModelScanRequest) =>
			apiFetch<ModelScanResult>('/api/setup/scan-models', {
				method: 'POST',
				body: JSON.stringify(request),
				headers: setupHeaders(sessionId, reconfigure)
			}),
		onSuccess: (data: ModelScanResult) => queryClient.setQueryData(['setup', 'scan-models'], data)
	}));
}

export function createRecommendationMutation(sessionId: string, reconfigure: boolean) {
	return createMutation(() => ({
		mutationFn: (request: RecommendationRequest) =>
			apiFetch<RecommendationResult>('/api/setup/recommend', {
				method: 'POST',
				body: JSON.stringify(request),
				headers: setupHeaders(sessionId, reconfigure)
			}),
		onSuccess: (data: RecommendationResult) =>
			queryClient.setQueryData(['setup', 'recommend'], data)
	}));
}

export interface PreviewConfigResponse {
	toml: string;
}

export function createPreviewConfigMutation(sessionId: string, reconfigure: boolean) {
	return createMutation(() => ({
		mutationFn: (request: SaveConfigRequest) =>
			apiFetch<PreviewConfigResponse>('/api/setup/preview-config', {
				method: 'POST',
				body: JSON.stringify(request),
				headers: setupHeaders(sessionId, reconfigure)
			})
	}));
}

export interface SaveConfigResponse {
	path: string;
	replaced_existing: boolean;
	backup_path?: string;
}

export function createSetupSaveConfigMutation(sessionId: string, reconfigure: boolean) {
	return createMutation(() => ({
		mutationFn: (request: SaveConfigRequest) =>
			apiFetch<SaveConfigResponse>('/api/setup/save-config', {
				method: 'POST',
				body: JSON.stringify(request),
				headers: setupHeaders(sessionId, reconfigure)
			})
	}));
}

export interface BuildCreateRequest {
	backend: SetupBackend;
	install_dir: string;
	jobs: number;
}

export interface BuildCreateResponse {
	build_id: string;
}

export function createBuildMutation(sessionId: string, reconfigure: boolean) {
	return createMutation(() => ({
		mutationFn: (request: BuildCreateRequest) =>
			apiFetch<BuildCreateResponse>('/api/setup/build', {
				method: 'POST',
				body: JSON.stringify(request),
				headers: setupHeaders(sessionId, reconfigure)
			})
	}));
}

export interface ValidateLaunchResponse {
	success: boolean;
	message: string;
	stderr?: string;
}

export function createValidateLaunchMutation(sessionId: string, reconfigure: boolean) {
	return createMutation(() => ({
		mutationFn: (servers: ServerConfigDraft[]) =>
			apiFetch<ValidateLaunchResponse>('/api/setup/validate-launch', {
				method: 'POST',
				body: JSON.stringify({ servers }),
				headers: setupHeaders(sessionId, reconfigure)
			})
	}));
}

export interface SetupStatus {
	setup_required: boolean;
	version: string;
}

/** Unauthenticated status used by the completion screen's poll (§13.6). */
export function fetchSetupStatus(): Promise<SetupStatus> {
	return apiFetch<SetupStatus>('/api/status');
}
