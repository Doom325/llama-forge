import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import CompletionScreen from './CompletionScreen.svelte';

describe('CompletionScreen', () => {
	it('polling shows a live status message', () => {
		render(CompletionScreen, { state: 'polling' });
		expect(screen.getByRole('status')).toBeInTheDocument();
	});

	it('success shows the running badge + Go to Dashboard', async () => {
		const ongotodashboard = vi.fn();
		render(CompletionScreen, { state: 'success', ongotodashboard });
		expect(screen.getByText('LlamaForge is running')).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: 'Go to Dashboard' }));
		expect(ongotodashboard).toHaveBeenCalledOnce();
	});

	it('timeout shows Retry + Go anyway', async () => {
		const onretry = vi.fn();
		render(CompletionScreen, { state: 'timeout', onretry, ongotodashboard: vi.fn() });
		expect(screen.getByRole('button', { name: 'Go to Dashboard anyway' })).toBeInTheDocument();
		await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
		expect(onretry).toHaveBeenCalledOnce();
	});
});
