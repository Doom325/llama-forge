export type ServerStatus =
	| 'healthy'
	| 'degraded'
	| 'failed'
	| 'pending'
	| 'reconnecting'
	| 'disconnected'
	| 'running'
	| 'stopped'
	| 'restarting';

export interface ServerState {
	id: string;
	name: string;
	status: ServerStatus;
	uptime: number;
}

export interface ServerMetricsSummary {
	throughput: number;
	latency: { p50: number; p90: number; p99: number };
	errorRate: number;
	slots: { total: number; idle: number; processing: number; loading: number };
}

export interface QueueItem {
	id: string;
	position: number;
	model: string;
	requestor: string;
	status: string;
	waitTime: number;
}

export interface RequestRecord {
	id: string;
	timestamp: string;
	model: string;
	taskType: string;
	duration: number;
	status: string;
}

export interface ModelInfo {
	id: string;
	name: string;
	loaded: boolean;
	assignedServers: string[];
	capabilities: string[];
}

export interface TimeSeriesPoint {
	timestamp: number;
	value: number;
}

export interface LatencyPercentilePoint {
	timestamp: number;
	p50: number;
	p90: number;
	p99: number;
}

export interface AggregatedMetrics {
	throughput_total: number;
	latency?: {
		p50: number;
		p90: number;
		p99: number;
	};
}

export interface GenerationParams {
	temperature: number;
	max_tokens: number;
	top_p: number;
	top_k: number;
}

// Playground chat / streaming (FRONTEND_ARCHITECTURE.md §5.2, §5.4, §6.4).
export interface ResponseMetrics {
	/** Time to first token, milliseconds. */
	ttft: number;
	tokensPerSecond: number;
	totalTokens: number;
}

export interface ChatError {
	message: string;
	retryable: boolean;
}

export interface Message {
	role: 'user' | 'assistant';
	content: string;
	timestamp: number;
	metrics?: ResponseMetrics;
}

/** Streaming lifecycle as a discriminated union — impossible states unrepresentable. */
export type StreamingState =
	| { status: 'idle' }
	| { status: 'queued'; position: number; eta: number | null }
	| { status: 'streaming'; tokens: string; ttft: number }
	| { status: 'complete'; response: string; metrics: ResponseMetrics }
	| { status: 'error'; error: ChatError; partialResponse: string | null };

export interface ChatCompletionRequest {
	model: string;
	messages: { role: 'user' | 'assistant' | 'system'; content: string }[];
	temperature: number;
	max_tokens: number;
	top_p: number;
	top_k: number;
	stream: true;
}

export type ConfigSection = 'servers' | 'priority' | 'global';

// Configuration shapes. These mirror the backend `llamaforge-core` config
// structs (Section 2.6). Frontend validation is intentionally loose — the
// backend is the authoritative validator (see FRONTEND_ARCHITECTURE.md §6.6).
export type LogLevel = 'trace' | 'debug' | 'info' | 'warn' | 'error';
export type LogFormat = 'pretty' | 'json' | 'compact';

export interface GlobalSettings {
	bind_address: string;
	grpc_port: number;
	http_port: number;
	log_level: LogLevel;
	log_format: LogFormat;
	metrics_retention_days: number;
	queue_max_depth: number;
	queue_default_timeout_secs: number;
	restart_stagger_secs: number;
}

export interface ServerConfigItem {
	id: string;
	alias: string;
	model_path: string;
	port: number;
	gpu_layers: number;
	parallel_slots: number;
	ctx_size: number;
}

export interface ServersConfig {
	servers: ServerConfigItem[];
}

export interface PriorityRule {
	task_type: string;
	priority: number;
}

export interface FairShareWeight {
	requestor: string;
	weight: number;
}

export interface PriorityRulesConfig {
	task_types: PriorityRule[];
	fair_share: FairShareWeight[];
	starvation: {
		boost_start_after_secs: number;
		max_boost: number;
	};
}

export type ServerAction = 'restart' | 'stop' | 'start';
export type DetailServerAction = ServerAction | 'force_kill';

export function getAvailableActions(status: ServerStatus): ServerAction[] {
	if (status === 'running' || status === 'healthy' || status === 'degraded') {
		return ['restart', 'stop'];
	}
	if (status === 'stopped' || status === 'failed' || status === 'disconnected') {
		return ['start'];
	}
	// 'restarting' or 'pending'
	return [];
}

export function getDetailAvailableActions(status: ServerStatus): DetailServerAction[] {
	const actions = getAvailableActions(status) as DetailServerAction[];
	if (status !== 'stopped' && status !== 'disconnected') {
		actions.push('force_kill');
	}
	return actions;
}

export function computeFleetHealth(servers: ServerState[]) {
	const counts = { healthy: 0, degraded: 0, failed: 0, total: servers.length };
	let allPending = servers.length > 0;

	for (const s of servers) {
		if (s.status === 'healthy' || s.status === 'running') counts.healthy++;
		else if (s.status === 'degraded' || s.status === 'reconnecting') counts.degraded++;
		else if (s.status === 'failed' || s.status === 'disconnected') counts.failed++;

		if (s.status !== 'pending' && s.status !== 'restarting') allPending = false;
	}

	let status: 'healthy' | 'degraded' | 'failed' = 'healthy';
	if (counts.failed > 0) status = 'failed';
	else if (
		counts.degraded > 0 ||
		(counts.healthy < counts.total && counts.healthy > 0) ||
		allPending
	)
		status = 'degraded';

	return { status, counts };
}

// ---------------------------------------------------------------------------
// Setup Wizard (08_SETUP_WIZARD.md). NOTE: /api/setup/* responses use a FLAT
// envelope — the backend adds a top-level `version: 1` field *alongside* the
// payload's own fields (not a nested `{ version, data }`). Queries therefore
// type the inner shape directly and ignore the extra `version` field.
// ---------------------------------------------------------------------------

// --- Hardware Detection ---
export interface GpuInfo {
	index: number;
	name: string;
	vram_mb: number;
}

export interface CpuInfo {
	name: string;
	cores_physical: number;
	cores_logical: number;
}

export type SetupBackend = 'cuda' | 'metal' | 'vulkan' | 'cpu';

export interface HardwareInfo {
	gpus: GpuInfo[];
	cpu: CpuInfo;
	ram_mb: number;
	recommended_backend: SetupBackend;
}

// --- Binary Scanner (discriminated on `found`) ---
export type BinaryScanResult =
	| { found: true; path: string; binary_version: string; backends: string[] }
	| { found: false; searched_paths: string[] };

// --- Model Scanner ---
export interface ModelScanRequest {
	source: 'local';
	directories: string[];
	max_depth?: number;
	max_results?: number;
}

export interface DiscoveredModel {
	id: string;
	path: string;
	filename: string;
	size_bytes: number;
	quantization_hint: string;
	architecture_hint: string;
	modified: string;
}

export type TruncationReason = 'timeout' | 'max_results' | 'max_depth' | null;

export interface ModelScanResult {
	models: DiscoveredModel[];
	scan_duration_ms: number;
	files_scanned: number;
	truncated: boolean;
	truncation_reason: TruncationReason;
	warnings: string[];
}

// --- Build (discriminated on `status`) ---
export type BuildPhase = 'cloning' | 'configuring' | 'building';

export type BuildStatus =
	| { status: 'idle' }
	| {
			status: 'running';
			build_id: string;
			phase: BuildPhase;
			percent: number | null;
			logLines: string[];
	  }
	| {
			status: 'complete';
			build_id: string;
			binary_path: string;
			duration_secs: number;
			log_path?: string;
			replaced_existing?: boolean;
			backup_path?: string | null;
	  }
	| {
			status: 'failed';
			build_id: string;
			phase: BuildPhase;
			error: string;
			lastLines: string[];
			log_path?: string;
	  }
	| { status: 'cancelled'; build_id: string };

// --- Config Recommendation ---
export interface RecommendationRequest {
	models: Array<{ path: string; size_bytes: number }>;
}

export interface ConfigRecommendation {
	gpu_layers: number;
	ctx_size: number;
	parallel_slots: number;
	cache_type_k: string;
	cache_type_v: string;
	flash_attn: string;
	estimated_vram_mb: number;
	port: number;
}

export interface ServerRecommendation {
	model_path: string;
	recommended: ConfigRecommendation;
	rationale: Record<string, string>;
}

export interface RecommendationResult {
	servers: ServerRecommendation[];
	warnings: string[];
}

// --- Server config draft (client-side, merged from recommendation + overrides) ---
export interface ServerConfigDraft {
	model_path: string;
	alias: string;
	binary_path: string;
	port: number;
	gpu_layers: number;
	parallel_slots: number;
	ctx_size: number;
	cache_type_k: string;
	cache_type_v: string;
	flash_attn: string;
	estimated_vram_mb: number;
}

// --- Save Config ---
export interface SaveConfigRequest {
	path: string;
	force: boolean;
	servers: ServerConfigDraft[];
	global_overrides?: {
		bind_address?: string;
		http_port?: number;
	};
}

// --- Wizard navigation ---
export type WizardStep = 'welcome' | 'hardware' | 'binary' | 'models' | 'configure' | 'review';
