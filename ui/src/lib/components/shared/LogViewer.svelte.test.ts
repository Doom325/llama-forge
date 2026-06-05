import { render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, it, expect } from 'vitest';
import LogViewer from './LogViewer.svelte';

describe('LogViewer', () => {
	it('renders lines in a focusable role=log live region', () => {
		render(LogViewer, { lines: ['line one', 'line two'], label: 'Build log' });
		const log = screen.getByRole('log');
		expect(log).toHaveAttribute('aria-live', 'polite');
		expect(log).toHaveAttribute('tabindex', '0');
		expect(log).toHaveAttribute('aria-label', 'Build log');
		expect(log.textContent).toContain('line one');
		expect(log.textContent).toContain('line two');
	});

	it('auto-tails to the bottom when new lines arrive', async () => {
		const { rerender } = render(LogViewer, { lines: ['a'] });
		const log = screen.getByRole('log') as HTMLPreElement;
		// jsdom does no layout, so fake a viewport taller than its content.
		Object.defineProperty(log, 'scrollHeight', { value: 1000, configurable: true });
		Object.defineProperty(log, 'clientHeight', { value: 400, configurable: true });

		await rerender({ lines: Array.from({ length: 100 }, (_, i) => `line ${i}`) });
		await tick();

		expect(log.scrollTop).toBe(1000);
	});
});
