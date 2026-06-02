import { describe, it, expect } from 'vitest';
import { getAvailableActions, getDetailAvailableActions } from './types';

describe('API Types', () => {
	it('maps status to available actions correctly', () => {
		expect(getAvailableActions('running')).toEqual(['restart', 'stop']);
		expect(getAvailableActions('stopped')).toEqual(['start']);
		expect(getAvailableActions('failed')).toEqual(['start']);
		expect(getAvailableActions('restarting')).toEqual([]);
	});

	it('adds force_kill to detail actions when appropriate', () => {
		expect(getDetailAvailableActions('running')).toEqual(['restart', 'stop', 'force_kill']);
		expect(getDetailAvailableActions('stopped')).toEqual(['start']);
		expect(getDetailAvailableActions('failed')).toEqual(['start', 'force_kill']);
		expect(getDetailAvailableActions('restarting')).toEqual(['force_kill']);
	});
});
