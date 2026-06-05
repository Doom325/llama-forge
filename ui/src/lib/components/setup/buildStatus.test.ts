import { describe, it, expect } from 'vitest';
import { applyBuildEvent } from './buildStatus';
import type { BuildStatus } from '$lib/api/types';

const IDLE: BuildStatus = { status: 'idle' };
const ID = 'build-1';

describe('applyBuildEvent', () => {
	it('phase from idle starts a running build', () => {
		const next = applyBuildEvent(IDLE, ID, {
			event: 'phase',
			data: { phase: 'cloning', message: 'Cloning…' }
		});
		expect(next).toMatchObject({
			status: 'running',
			build_id: ID,
			phase: 'cloning',
			percent: null
		});
	});

	it('progress sets phase + percent', () => {
		const running = applyBuildEvent(IDLE, ID, {
			event: 'phase',
			data: { phase: 'building', message: '' }
		});
		const next = applyBuildEvent(running, ID, {
			event: 'progress',
			data: { phase: 'building', percent: 50 }
		});
		expect(next).toMatchObject({ status: 'running', phase: 'building', percent: 50 });
	});

	it('log accumulates lines across events', () => {
		let s = applyBuildEvent(IDLE, ID, { event: 'log', data: { lines: ['a', 'b'] } });
		s = applyBuildEvent(s, ID, { event: 'log', data: { lines: ['c'] } });
		expect(s.status).toBe('running');
		if (s.status === 'running') expect(s.logLines).toEqual(['a', 'b', 'c']);
	});

	it('complete replaces the status with terminal success', () => {
		const next = applyBuildEvent(IDLE, ID, {
			event: 'complete',
			data: {
				binary_path: '/x/llama-server',
				duration_secs: 12,
				replaced_existing: true,
				backup_path: '/x/llama-server.backup',
				log_path: '/logs/build.log'
			}
		});
		expect(next).toEqual({
			status: 'complete',
			build_id: ID,
			binary_path: '/x/llama-server',
			duration_secs: 12,
			replaced_existing: true,
			backup_path: '/x/llama-server.backup',
			log_path: '/logs/build.log'
		});
	});

	it('failed maps last_lines → lastLines and carries the error', () => {
		const next = applyBuildEvent(IDLE, ID, {
			event: 'failed',
			data: { phase: 'building', error: 'cmake failed', last_lines: ['err1', 'err2'] }
		});
		expect(next).toMatchObject({
			status: 'failed',
			phase: 'building',
			error: 'cmake failed',
			lastLines: ['err1', 'err2']
		});
	});

	it('defaults missing backup_path to null on complete', () => {
		const next = applyBuildEvent(IDLE, ID, {
			event: 'complete',
			data: { binary_path: '/x', duration_secs: 1, replaced_existing: false }
		});
		if (next.status === 'complete') expect(next.backup_path).toBeNull();
	});
});
