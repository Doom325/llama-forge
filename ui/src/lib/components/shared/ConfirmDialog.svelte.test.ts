import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ConfirmDialog from './ConfirmDialog.svelte';
import userEvent from '@testing-library/user-event';

describe('ConfirmDialog', () => {
	it('fires onConfirm on confirm click and closes', async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn();
		const onCancel = vi.fn();

		render(ConfirmDialog, { open: true, title: 'Title', message: 'Message', onConfirm, onCancel });

		expect(screen.getByRole('dialog')).toBeInTheDocument();
		await user.click(screen.getByRole('button', { name: 'Confirm' }));

		expect(onConfirm).toHaveBeenCalledOnce();
		expect(onCancel).not.toHaveBeenCalled();
	});

	it('fires onCancel on cancel click and closes', async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn();
		const onCancel = vi.fn();

		render(ConfirmDialog, { open: true, title: 'Title', message: 'Message', onConfirm, onCancel });

		await user.click(screen.getByRole('button', { name: 'Cancel' }));

		expect(onCancel).toHaveBeenCalledOnce();
		expect(onConfirm).not.toHaveBeenCalled();
	});
});
