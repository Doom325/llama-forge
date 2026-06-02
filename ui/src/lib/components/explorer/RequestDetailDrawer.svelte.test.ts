import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import RequestDetailDrawer from './RequestDetailDrawer.svelte';
import userEvent from '@testing-library/user-event';

describe('RequestDetailDrawer', () => {
	const mockRequest = {
		id: '1',
		timestamp: '2023-01-01T12:00:00Z',
		model: 'llama3',
		taskType: 'chat',
		duration: 1500,
		status: 'success'
	};

	it('renders content when open and focuses close button', async () => {
		render(RequestDetailDrawer, { request: mockRequest, open: true, onClose: vi.fn() });

		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByText('Request Details')).toBeInTheDocument();
		expect(screen.getByText('llama3')).toBeInTheDocument();

		// Svelte 5 effect delay requires tick/timeout
		await new Promise((r) => setTimeout(r, 100));

		const closeBtn = screen.getByRole('button', { name: 'Close details' });
		expect(document.activeElement).toBe(closeBtn);
	});

	it('calls onClose when escape is pressed', async () => {
		const user = userEvent.setup();
		const onClose = vi.fn();
		render(RequestDetailDrawer, { request: mockRequest, open: true, onClose });

		await user.keyboard('{Escape}');
		expect(onClose).toHaveBeenCalled();
	});
});
