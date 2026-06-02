import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { toastStore } from './toast.svelte';

describe('Toast Store', () => {
	beforeEach(() => {
		vi.useFakeTimers();
		toastStore.toasts = []; // reset
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it('adds toasts and caps at 3 visible', () => {
		toastStore.add('info', 'Toast 1');
		toastStore.add('info', 'Toast 2');
		toastStore.add('info', 'Toast 3');
		toastStore.add('info', 'Toast 4');

		expect(toastStore.toasts.length).toBe(3);
		expect(toastStore.toasts[0]?.message).toBe('Toast 2');
		expect(toastStore.toasts[2]?.message).toBe('Toast 4');
	});

	it('auto-dismisses toasts after 5 seconds', () => {
		toastStore.add('success', 'Temp');
		expect(toastStore.toasts.length).toBe(1);

		vi.advanceTimersByTime(5000);
		expect(toastStore.toasts.length).toBe(0);
	});
});
