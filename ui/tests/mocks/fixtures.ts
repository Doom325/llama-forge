import type { Page, Route } from '@playwright/test';
import type {
	ServerState,
	ServerStatus,
	ServerMetricsSummary,
	QueueItem,
	RequestRecord,
	ModelInfo,
	GlobalSettings,
	ServersConfig,
	PriorityRulesConfig,
	AggregatedMetrics
} from '../../src/lib/api/types';

// Realistic fixture data backing the Playwright E2E suite (design §11.2). All
// route handlers are deterministic so visual snapshots stay stable — volatile
// regions (timestamps, live metrics) are masked in the visual spec instead.

const STATUSES: ServerStatus[] = [
	'healthy',
	'healthy',
	'healthy',
	'degraded',
	'healthy',
	'failed',
	'healthy',
	'reconnecting'
];

export const servers: ServerState[] = Array.from({ length: 8 }, (_, i) => ({
	id: `srv-${String(i + 1).padStart(2, '0')}`,
	name: `llama-node-${i + 1}`,
	status: STATUSES[i] ?? 'healthy',
	// Deterministic uptime so the dashboard renders identically every run.
	uptime: 3600 * (i + 1) + 120 * i
}));

export const serverMetrics: ServerMetricsSummary = {
	throughput: 142.5,
	latency: { p50: 120, p90: 340, p99: 980 },
	errorRate: 0.012,
	slots: { total: 8, idle: 3, processing: 4, loading: 1 }
};

export const queue: QueueItem[] = [
	{
		id: 'q-1',
		position: 1,
		model: 'llama3-8b',
		requestor: 'svc-chat',
		status: 'waiting',
		waitTime: 1200
	},
	{
		id: 'q-2',
		position: 2,
		model: 'mistral-7b',
		requestor: 'svc-batch',
		status: 'waiting',
		waitTime: 800
	},
	{
		id: 'q-3',
		position: 3,
		model: 'llama3-8b',
		requestor: 'svc-chat',
		status: 'waiting',
		waitTime: 400
	}
];

const MODEL_IDS = ['llama3-8b', 'mistral-7b', 'qwen2-7b'];
const TASK_TYPES = ['chat', 'completion', 'embedding'];
const REQUEST_STATUSES = ['success', 'success', 'success', 'error', 'pending'];

// ~1000 rows exercises the virtualized table at realistic volume. Generated
// from a fixed seed (the index) so the data set is identical across runs.
export const requests: RequestRecord[] = Array.from({ length: 1000 }, (_, i) => ({
	id: `req-${String(i + 1).padStart(5, '0')}`,
	// Fixed epoch base; each row 1s apart. Masked in visual tests.
	timestamp: new Date(1_700_000_000_000 + i * 1000).toISOString(),
	model: MODEL_IDS[i % MODEL_IDS.length]!,
	taskType: TASK_TYPES[i % TASK_TYPES.length]!,
	duration: 200 + (i % 50) * 37,
	status: REQUEST_STATUSES[i % REQUEST_STATUSES.length]!
}));

export const models: ModelInfo[] = [
	{
		id: 'llama3-8b',
		name: 'Llama 3 8B',
		loaded: true,
		assignedServers: ['srv-01', 'srv-02'],
		capabilities: ['chat', 'completion']
	},
	{
		id: 'mistral-7b',
		name: 'Mistral 7B',
		loaded: true,
		assignedServers: ['srv-03'],
		capabilities: ['chat', 'completion']
	},
	{
		id: 'qwen2-7b',
		name: 'Qwen2 7B',
		loaded: false,
		assignedServers: [],
		capabilities: ['chat', 'embedding']
	}
];

export const globalConfig: GlobalSettings = {
	bind_address: '0.0.0.0',
	grpc_port: 50051,
	http_port: 8080,
	log_level: 'info',
	log_format: 'pretty',
	metrics_retention_days: 30,
	queue_max_depth: 256,
	queue_default_timeout_secs: 60,
	restart_stagger_secs: 5
};

export const serversConfig: ServersConfig = {
	servers: [
		{
			id: 'srv-01',
			alias: 'llama-node-1',
			model_path: '/models/llama3-8b.gguf',
			port: 9001,
			gpu_layers: 35,
			parallel_slots: 8,
			ctx_size: 8192
		},
		{
			id: 'srv-02',
			alias: 'llama-node-2',
			model_path: '/models/mistral-7b.gguf',
			port: 9002,
			gpu_layers: 32,
			parallel_slots: 4,
			ctx_size: 4096
		}
	]
};

export const priorityConfig: PriorityRulesConfig = {
	task_types: [
		{ task_type: 'chat', priority: 10 },
		{ task_type: 'completion', priority: 5 },
		{ task_type: 'embedding', priority: 1 }
	],
	fair_share: [
		{ requestor: 'svc-chat', weight: 3 },
		{ requestor: 'svc-batch', weight: 1 }
	],
	starvation: { boost_start_after_secs: 30, max_boost: 5 }
};

export const aggregatedMetrics: AggregatedMetrics = {
	throughput_total: 1024.5,
	latency: { p50: 110, p90: 320, p99: 910 }
};

const CONFIG_BY_SECTION: Record<string, unknown> = {
	global: globalConfig,
	servers: serversConfig,
	priority: priorityConfig
};

// A short markdown reply streamed token-by-token as SSE deltas. Kept brief so
// the playground spec finishes quickly while still rendering real markdown.
const CHAT_REPLY =
	'Here is a **streamed** reply with a list:\n\n- first point\n- second point\n\nDone.';

function sseChatBody(reply: string): string {
	// Chunk the reply into a handful of deltas, mirroring the OpenAI streaming
	// frame shape the client parses in `chat.ts`.
	const words = reply.split(/(\s+)/).filter(Boolean);
	const frames = words.map(
		(w) => `data: ${JSON.stringify({ choices: [{ delta: { content: w } }] })}\n\n`
	);
	frames.push('data: [DONE]\n\n');
	return frames.join('');
}

/**
 * Registers every API route handler the app calls onto a Playwright `page`.
 * Mirrors the API contract in `src/lib/api/queries.ts`. Call once at the top of
 * each E2E test before `page.goto(...)`.
 *
 * Uses Playwright route interception (the established convention — NOT MSW).
 */
export async function mockApi(page: Page): Promise<void> {
	const json = (route: Route, data: unknown, headers: Record<string, string> = {}) =>
		route.fulfill({
			status: 200,
			contentType: 'application/json',
			headers,
			body: JSON.stringify(data)
		});

	// Server detail / metrics — more specific patterns first.
	await page.route('**/api/servers/*/metrics', (route) => json(route, serverMetrics));
	await page.route('**/api/servers/*', (route) => {
		const url = new URL(route.request().url());
		const id = url.pathname.split('/').pop() ?? '';
		// POST actions (restart/stop/start/force_kill) return 204.
		if (route.request().method() !== 'GET') {
			return route.fulfill({ status: 204, body: '' });
		}
		const match = servers.find((s) => s.id === id) ?? servers[0];
		return json(route, match);
	});
	await page.route('**/api/servers', (route) => json(route, servers));

	await page.route('**/api/queue', (route) => json(route, queue));
	await page.route('**/api/models', (route) => json(route, models));

	await page.route('**/api/requests*', (route) => {
		if (route.request().method() !== 'GET') {
			return route.fulfill({ status: 204, body: '' });
		}
		const url = new URL(route.request().url());
		const model = url.searchParams.get('model');
		const taskType = url.searchParams.get('taskType');
		let rows = requests;
		if (model) rows = rows.filter((r) => r.model === model);
		if (taskType) rows = rows.filter((r) => r.taskType === taskType);
		return json(route, rows);
	});

	await page.route('**/api/metrics**', (route) => json(route, aggregatedMetrics));

	// Config GET/PUT — echoes an ETag header (optimistic concurrency).
	await page.route('**/api/config/*', (route) => {
		const url = new URL(route.request().url());
		const section = url.pathname.split('/').pop() ?? 'global';
		const data = CONFIG_BY_SECTION[section] ?? globalConfig;
		const etag = `"etag-${section}-v1"`;
		if (route.request().method() !== 'GET') {
			// PUT echoes the saved body back with a bumped ETag.
			let body: unknown = data;
			try {
				body = JSON.parse(route.request().postData() ?? 'null') ?? data;
			} catch {
				// keep default
			}
			return json(route, body, { ETag: `"etag-${section}-v2"` });
		}
		return json(route, data, { ETag: etag });
	});

	// SSE event stream — keep it open but idle so the app reports "connected"
	// (no reconnecting chip) without emitting volatile updates.
	await page.route('**/api/events', (route) =>
		route.fulfill({
			status: 200,
			contentType: 'text/event-stream',
			headers: { 'Cache-Control': 'no-cache', Connection: 'keep-alive' },
			body: 'event: heartbeat\ndata: {}\n\n'
		})
	);

	// Streaming chat completion (playground).
	await page.route('**/v1/chat/completions', (route) =>
		route.fulfill({
			status: 200,
			contentType: 'text/event-stream',
			headers: { 'Cache-Control': 'no-cache' },
			body: sseChatBody(CHAT_REPLY)
		})
	);
}

export const chatReply = CHAT_REPLY;
