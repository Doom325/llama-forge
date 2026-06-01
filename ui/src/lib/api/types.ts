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

export interface GenerationParams {
	temperature: number;
	max_tokens: number;
	top_p: number;
	top_k: number;
}

export type ConfigSection = 'servers' | 'priority' | 'global';

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
