export const API_BASE = '/api';
export const SSE_ENDPOINT = `${API_BASE}/events`;

export const ROUTES = {
	DASHBOARD: '/',
	SERVER_DETAIL: (id: string) => `/servers/${id}`,
	QUEUE: '/queue',
	EXPLORER: '/explorer',
	CONFIG_SERVERS: '/config/servers',
	CONFIG_PRIORITY: '/config/priority',
	PLAYGROUND: '/playground',
	MODELS: '/models'
};

export const DEFAULT_TEMPERATURE = 0.7;
export const DEFAULT_MAX_TOKENS = 2048;
export const DEFAULT_TOP_P = 0.9;
export const DEFAULT_TOP_K = 40;

export const TIMEOUTS = {
	GET: 10_000,
	MUTATION: 30_000,
	SSE_HEARTBEAT: 30_000,
	PLAYGROUND_ABORT: 120_000
};
