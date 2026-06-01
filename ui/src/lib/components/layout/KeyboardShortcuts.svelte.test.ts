import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import KeyboardShortcuts from './KeyboardShortcuts.svelte';
import userEvent from '@testing-library/user-event';

describe('KeyboardShortcuts', () => {
	it('opens dialog on button click', async () => {
		const user = userEvent.setup();
		render(KeyboardShortcuts);

		const triggerBtn = screen.getByRole('button', { name: 'Keyboard shortcuts' });
		await user.click(triggerBtn);

		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByText('Toggle Sidebar')).toBeInTheDocument();
	});

	it('opens dialog on ? keydown', async () => {
		const user = userEvent.setup();
		render(KeyboardShortcuts);

		await user.keyboard('?');

		expect(screen.getByRole('dialog')).toBeInTheDocument();
	});
});
