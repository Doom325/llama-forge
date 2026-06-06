import { describe, it, expect } from 'vitest';
import { QueryClient } from '@tanstack/svelte-query';
import {
	SetupWizardStore,
	STEP_TRANSITIONS,
	completedSteps,
	invalidateDownstream
} from './setup.svelte';

function qc(): QueryClient {
	return new QueryClient();
}

describe('SetupWizardStore', () => {
	it('starts empty with the given session id', () => {
		const store = new SetupWizardStore('sess-1');
		expect(store.sessionId).toBe('sess-1');
		expect(store.selectedModelIds.size).toBe(0);
		expect(store.userOverrides.size).toBe(0);
		expect(store.savePath).toBe('');
		expect(store.reconfigure).toBe(false);
	});

	it('carries the reconfigure flag (Flow 4)', () => {
		expect(new SetupWizardStore('s', true).reconfigure).toBe(true);
	});
});

describe('STEP_TRANSITIONS guards', () => {
	it('welcome and review always advance', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		expect(STEP_TRANSITIONS.welcome.canAdvance(c, store)).toBe(true);
		expect(STEP_TRANSITIONS.review.canAdvance(c, store)).toBe(true);
	});

	it('hardware advances only once hardware data is cached', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		expect(STEP_TRANSITIONS.hardware.canAdvance(c, store)).toBe(false);
		c.setQueryData(['setup', 'hardware'], {
			gpus: [],
			cpu: { name: 'x', cores_physical: 1, cores_logical: 1 },
			ram_mb: 1024,
			recommended_backend: 'cpu'
		});
		expect(STEP_TRANSITIONS.hardware.canAdvance(c, store)).toBe(true);
	});

	it('binary advances only when a binary was found', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		c.setQueryData(['setup', 'scan-binary'], { found: false, searched_paths: [] });
		expect(STEP_TRANSITIONS.binary.canAdvance(c, store)).toBe(false);
		c.setQueryData(['setup', 'scan-binary'], {
			found: true,
			path: '/usr/bin/llama-server',
			binary_version: 'b1',
			backends: ['cpu']
		});
		expect(STEP_TRANSITIONS.binary.canAdvance(c, store)).toBe(true);
	});

	it('models advances once a model is selected', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		expect(STEP_TRANSITIONS.models.canAdvance(c, store)).toBe(false);
		store.selectedModelIds.add('m1');
		expect(STEP_TRANSITIONS.models.canAdvance(c, store)).toBe(true);
	});

	it('configure advances on accept-all (valid merged config, no edits) and blocks a bad edit', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		// No selection / no recommendation yet → cannot advance.
		expect(STEP_TRANSITIONS.configure.canAdvance(c, store)).toBe(false);

		c.setQueryData(['setup', 'scan-models'], {
			models: [
				{
					id: 'm1',
					path: '/a.gguf',
					filename: 'a.gguf',
					size_bytes: 1,
					quantization_hint: '',
					architecture_hint: ''
				}
			]
		});
		c.setQueryData(['setup', 'recommend'], {
			servers: [
				{
					model_path: '/a.gguf',
					recommended: {
						gpu_layers: 33,
						ctx_size: 4096,
						parallel_slots: 4,
						cache_type_k: 'f16',
						cache_type_v: 'f16',
						flash_attn: 'auto',
						estimated_vram_mb: 8000,
						port: 8001
					},
					rationale: {}
				}
			],
			warnings: []
		});
		store.selectedModelIds.add('m1');

		// Accept-all: no overrides, but the merged config is valid → advance (G1).
		expect(STEP_TRANSITIONS.configure.canAdvance(c, store)).toBe(true);

		// A genuinely invalid edit blocks advancement.
		store.userOverrides.set('m1', { port: 0 });
		expect(STEP_TRANSITIONS.configure.canAdvance(c, store)).toBe(false);
	});
});

describe('completedSteps', () => {
	it('is derived from the guards, not stored', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		const done = completedSteps(c, store);
		expect(done.has('welcome')).toBe(true);
		expect(done.has('review')).toBe(true);
		expect(done.has('hardware')).toBe(false);
		expect(done.has('models')).toBe(false);
	});
});

describe('invalidateDownstream', () => {
	it('clears overrides and removes recommend data when configure is invalidated', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		store.userOverrides.set('m1', { port: 8001 });
		c.setQueryData(['setup', 'recommend'], { servers: [], warnings: [] });

		// Re-entering hardware invalidates ['configure', 'review'].
		invalidateDownstream(c, store, 'hardware');

		expect(store.userOverrides.size).toBe(0);
		expect(c.getQueryData(['setup', 'recommend'])).toBeUndefined();
	});

	it('leaves state untouched for steps with no downstream', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		store.userOverrides.set('m1', { port: 8001 });
		invalidateDownstream(c, store, 'review');
		expect(store.userOverrides.size).toBe(1);
	});
});
