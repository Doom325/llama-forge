import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ChatPane from './ChatPane.svelte';
import type { Message, StreamingState } from '$lib/api/types';

const baseProps = {
	messages: [] as Message[],
	onSend: vi.fn(),
	onCancel: vi.fn(),
	onClear: vi.fn()
};

describe('ChatPane', () => {
	it('renders an enabled textarea and a Send button when idle', () => {
		const streamingState: StreamingState = { status: 'idle' };
		render(ChatPane, { ...baseProps, streamingState });

		expect(screen.getByLabelText('Message')).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Send' })).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
	});

	it('shows a Cancel button while streaming', () => {
		const streamingState: StreamingState = { status: 'streaming', tokens: 'hel', ttft: 12 };
		render(ChatPane, { ...baseProps, streamingState });

		expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
		expect(screen.queryByRole('button', { name: 'Send' })).not.toBeInTheDocument();
	});

	it('shows a Retry button and the partial response on error', () => {
		const streamingState: StreamingState = {
			status: 'error',
			error: { message: 'stream blew up', retryable: true },
			partialResponse: 'partial answer'
		};
		render(ChatPane, {
			...baseProps,
			messages: [{ role: 'user', content: 'hi', timestamp: 1 }],
			streamingState
		});

		expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
		expect(screen.getByText('stream blew up')).toBeInTheDocument();
		expect(screen.getByText('partial answer')).toBeInTheDocument();
	});
});
