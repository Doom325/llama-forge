import { describe, it, expect } from 'vitest';
import { formatDuration, formatTokensPerSecond, formatNumber, formatBytes } from './format';

describe('format utilities', () => {
	it('formats duration', () => {
		expect(formatDuration(125)).toBe('2m 5s');
		expect(formatDuration(45)).toBe('45s');
		expect(formatDuration(120)).toBe('2m');
		expect(formatDuration(0)).toBe('0s');
		expect(formatDuration(-10)).toBe('0s');
	});

	it('formats tokens per second', () => {
		expect(formatTokensPerSecond(42.7)).toBe('42.7 tok/s');
		expect(formatTokensPerSecond(NaN)).toBe('0 tok/s');
	});

	it('formats number', () => {
		expect(formatNumber(1000000)).toBe('1,000,000');
		expect(formatNumber(NaN)).toBe('0');
	});

	it('formats bytes', () => {
		expect(formatBytes(1024)).toBe('1 KB');
		expect(formatBytes(1500)).toBe('1.46 KB');
		expect(formatBytes(1048576)).toBe('1 MB');
		expect(formatBytes(0)).toBe('0 B');
		expect(formatBytes(NaN)).toBe('0 B');
	});
});
