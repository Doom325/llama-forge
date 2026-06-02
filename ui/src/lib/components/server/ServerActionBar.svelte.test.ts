import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ServerActionBar from './ServerActionBar.svelte';
import userEvent from '@testing-library/user-event';

describe('ServerActionBar', () => {
	it('renders available actions as a menu', () => {
		render(ServerActionBar, {
			availableActions: ['restart', 'stop', 'force_kill'],
			pendingAction: null,
			onAction: vi.fn()
		});

		expect(screen.getByRole('menu', { name: 'Server actions' })).toBeInTheDocument();
		const items = screen.getAllByRole('menuitem');
		expect(items.length).toBe(3);
		expect(items[0]).toHaveTextContent('Restart Server');
		expect(items[1]).toHaveTextContent('Stop Server');
		expect(items[2]).toHaveTextContent('Force Kill');
	});

	it('disables buttons when pendingAction is set and updates label', () => {
		const { rerender } = render(ServerActionBar, {
			availableActions: ['restart', 'stop'],
			pendingAction: null,
			onAction: vi.fn()
		});

		const restartBtn = screen.getByRole('menuitem', { name: 'Restart Server' });
		const stopBtn = screen.getByRole('menuitem', { name: 'Stop Server' });

		expect(restartBtn).not.toBeDisabled();
		expect(stopBtn).not.toBeDisabled();

		rerender({
			availableActions: ['restart', 'stop'],
			pendingAction: 'restart',
			onAction: vi.fn()
		});

		expect(screen.getByRole('menuitem', { name: 'Restarting...' })).toBeDisabled();
		expect(screen.getByRole('menuitem', { name: 'Stop Server' })).toBeDisabled(); // All disabled
	});

	it('retains focus on state transition', async () => {
		const user = userEvent.setup();
		const onAction = vi.fn();
		const { rerender } = render(ServerActionBar, {
			availableActions: ['restart', 'stop'],
			pendingAction: null,
			onAction
		});

		const stopBtn = screen.getByRole('menuitem', { name: 'Stop Server' });
		stopBtn.focus();
		expect(document.activeElement).toBe(stopBtn);

		await user.click(stopBtn);
		expect(onAction).toHaveBeenCalledWith('stop');

		// Transition to pending
		rerender({ availableActions: ['restart', 'stop'], pendingAction: 'stop', onAction });

		// The element should still have focus, but text should be Stopping...
		const pendingBtn = screen.getByRole('menuitem', { name: 'Stopping...' });
		expect(document.activeElement).toBe(pendingBtn);
		expect(pendingBtn).toBeDisabled();
	});
});
