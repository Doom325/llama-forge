import { describe, it, expect } from 'vitest';
import { QueryClient } from '@tanstack/svelte-query';
import {
	SetupWizardStore,
	STEP_TRANSITIONS,
	completedSteps,
	invalidateDownstream,
	allFieldsValid
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

	it('configure requires non-empty, valid overrides', () => {
		const c = qc();
		const store = new SetupWizardStore('x');
		expect(STEP_TRANSITIONS.configure.canAdvance(c, store)).toBe(false);
		store.userOverrides.set('m1', { port: 8001 });
		expect(STEP_TRANSITIONS.configure.canAdvance(c, store)).toBe(true);
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

describe('allFieldsValid', () => {
	it('rejects non-positive port, ctx_size, slots, and blank alias', () => {
		const store = new SetupWizardStore('x');
		store.userOverrides.set('m', { port: -1 });
		expect(allFieldsValid(store)).toBe(false);
		store.userOverrides.set('m', { ctx_size: 0 });
		expect(allFieldsValid(store)).toBe(false);
		store.userOverrides.set('m', { alias: '  ' });
		expect(allFieldsValid(store)).toBe(false);
		store.userOverrides.set('m', { port: 8001, alias: 'ok' });
		expect(allFieldsValid(store)).toBe(true);
	});
});
