import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import QueuePageWrapper from '../../lib/components/testing/QueuePageWrapper.svelte';
import userEvent from '@testing-library/user-event';
import * as queries from '$lib/api/queries';
import { toastStore } from '$lib/stores/toast.svelte';

describe('QueuePage', () => {
	const mockQueue = [
		{
			id: 'req-1',
			position: 1,
			model: 'llama3',
			requestor: 'admin',
			status: 'processing',
			waitTime: 120
		},
		{
			id: 'req-2',
			position: 2,
			model: 'llama3',
			requestor: 'user1',
			status: 'queued',
			waitTime: 60
		}
	];

	beforeEach(() => {
		vi.spyOn(queries, 'createQueueQuery').mockReturnValue({
			isPending: false,
			isError: false,
			data: mockQueue
		} as unknown as ReturnType<typeof queries.createQueueQuery>);

		toastStore.toasts = [];
	});

	it('optimistic cancel ui and recovery on error', async () => {
		const user = userEvent.setup();

		let mockMutateCb: ((error: Error) => void) | undefined;
		vi.spyOn(queries, 'createCancelRequestMutation').mockReturnValue({
			mutate: vi.fn((id, options) => {
				mockMutateCb = options?.onError;
			})
		} as unknown as ReturnType<typeof queries.createCancelRequestMutation>);

		render(QueuePageWrapper);

		const cancelBtn = screen.getByRole('button', { name: 'Cancel request req-2' });
		expect(cancelBtn).toHaveTextContent('Cancel');
		expect(cancelBtn).not.toBeDisabled();

		// Click cancel
		await user.click(cancelBtn);

		// Optimistic update kicks in
		expect(cancelBtn).toHaveTextContent('Cancelling...');
		expect(cancelBtn).toBeDisabled();

		// Simulate error from server
		if (mockMutateCb) {
			mockMutateCb(new Error('Network error'));
		}

		// Optimistic update reverted
		// NOTE: In Svelte 5, state updates trigger DOM synchronously in some tests, but we'll await just in case
		await new Promise((r) => setTimeout(r, 0));

		expect(cancelBtn).toHaveTextContent('Cancel');
		expect(cancelBtn).not.toBeDisabled();

		// Toast is shown
		expect(toastStore.toasts.length).toBe(1);
		expect(toastStore.toasts[0]?.message).toContain('Network error');
	});
});
