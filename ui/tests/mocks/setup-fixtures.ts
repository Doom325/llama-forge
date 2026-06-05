import type { Page, Route } from '@playwright/test';

// Deterministic `/api/setup/*` fixtures for the wizard E2E + a11y specs. All
// responses use the FLAT version envelope the backend emits (version alongside
// the payload fields, not nested under `data`).

const json = (route: Route, data: unknown, status = 200) =>
	route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) });

export const hardware = {
	version: 1,
	gpus: [{ index: 0, name: 'NVIDIA RTX 4090', vram_mb: 24576 }],
	cpu: { name: 'AMD Ryzen 9 7950X', cores_physical: 16, cores_logical: 32 },
	ram_mb: 65536,
	recommended_backend: 'cuda'
};

export const binaryFound = {
	version: 1,
	found: true,
	path: '/usr/local/bin/llama-server',
	binary_version: 'b4567',
	backends: ['cuda', 'cpu']
};

export const models = {
	version: 1,
	models: [
		{
			id: 'm1',
			path: '/models/llama-3-8b.gguf',
			filename: 'llama-3-8b-Q4_K_M.gguf',
			size_bytes: 4_400_000_000,
			quantization_hint: 'Q4_K_M',
			architecture_hint: 'llama',
			modified: '2024-01-15T00:00:00Z'
		}
	],
	scan_duration_ms: 12,
	files_scanned: 1,
	truncated: false,
	truncation_reason: null,
	warnings: []
};

export const recommendation = {
	version: 1,
	servers: [
		{
			model_path: '/models/llama-3-8b.gguf',
			recommended: {
				gpu_layers: 33,
				ctx_size: 8192,
				parallel_slots: 4,
				cache_type_k: 'f16',
				cache_type_v: 'f16',
				flash_attn: 'auto',
				estimated_vram_mb: 9000,
				port: 8080
			},
			rationale: { gpu_layers: 'All layers fit in 24 GB VRAM.' }
		}
	],
	warnings: []
};

export const previewToml = {
	version: 1,
	toml: '[global]\nbind_address = "127.0.0.1"\nhttp_port = 8000\n\n[servers.llama-3-8b]\nmodel_path = "/models/llama-3-8b.gguf"\nport = 8080\ngpu_layers = 33\n'
};

export const saveResult = {
	version: 1,
	path: '~/.config/llamaforge/llamaforge.toml',
	replaced_existing: false
};

export const validateResult = {
	version: 1,
	success: true,
	message: 'server started and passed its health check'
};

/**
 * Register `/api/setup/*` + `/api/status` route handlers driving the happy path:
 * binary found, one model, a valid recommendation, save + validate succeed, and
 * status already reports running so the completion screen resolves.
 */
export async function mockSetupApi(page: Page): Promise<void> {
	await page.route('**/api/status', (route) =>
		json(route, { setup_required: false, version: '0.1.0' })
	);
	await page.route('**/api/setup/hardware', (route) => json(route, hardware));
	await page.route('**/api/setup/scan-binary', (route) => json(route, binaryFound));
	await page.route('**/api/setup/scan-models', (route) => json(route, models));
	await page.route('**/api/setup/recommend', (route) => json(route, recommendation));
	await page.route('**/api/setup/preview-config', (route) => json(route, previewToml));
	await page.route('**/api/setup/save-config', (route) => json(route, saveResult));
	await page.route('**/api/setup/validate-launch', (route) => json(route, validateResult));
}
