import { describe, it, expect } from 'vitest';
import { setupHeaders } from './queries';

describe('setupHeaders', () => {
	it('always sends the session id', () => {
		expect(setupHeaders('abc', false)).toEqual({ 'X-Setup-Session-Id': 'abc' });
	});

	it('adds X-Setup-Reconfigure only in the reconfigure flow (#274 / §13.8)', () => {
		expect(setupHeaders('abc', true)).toEqual({
			'X-Setup-Session-Id': 'abc',
			'X-Setup-Reconfigure': 'true'
		});
	});
});
