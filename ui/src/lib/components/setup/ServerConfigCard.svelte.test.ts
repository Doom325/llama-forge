import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ServerConfigCard from './ServerConfigCard.svelte';
import type { ConfigRecommendation, DiscoveredModel, ServerConfigDraft } from '$lib/api/types';

const model: DiscoveredModel = {
	id: 'm1',
	path: '/models/a.gguf',
	filename: 'a.gguf',
	size_bytes: 1024,
	quantization_hint: 'Q4_K_M',
	architecture_hint: 'llama'
};

const recommendation: ConfigRecommendation = {
	gpu_layers: 33,
	ctx_size: 4096,
	parallel_slots: 4,
	cache_type_k: 'f16',
	cache_type_v: 'f16',
	flash_attn: 'auto',
	estimated_vram_mb: 8000,
	port: 8001
};

const values: ServerConfigDraft = {
	model_path: '/models/a.gguf',
	alias: 'a',
	binary_path: '/usr/bin/llama-server',
	port: 8001,
	gpu_layers: 33,
	parallel_slots: 4,
	ctx_size: 4096,
	cache_type_k: 'f16',
	cache_type_v: 'f16',
	flash_attn: 'auto',
	estimated_vram_mb: 8000
};

describe('ServerConfigCard', () => {
	it('emits onchange with the edited port', async () => {
		const onchange = vi.fn();
		render(ServerConfigCard, { model, recommendation, values, onchange });
		await fireEvent.input(screen.getByLabelText('Port'), { target: { value: '9000' } });
		expect(onchange).toHaveBeenCalledWith(expect.objectContaining({ port: 9000, alias: 'a' }));
	});

	it('flags an invalid port', async () => {
		const onchange = vi.fn();
		render(ServerConfigCard, { model, recommendation, values: { ...values, port: 0 }, onchange });
		expect(screen.getByText('Port must be 1–65535.')).toBeInTheDocument();
	});

	it('shows the recommended gpu layers hint', () => {
		render(ServerConfigCard, { model, recommendation, values, onchange: vi.fn() });
		expect(screen.getByText('Recommended: 33')).toBeInTheDocument();
	});
});
