import { describe, it, expect, vi, afterEach } from 'vitest';
import { streamChatCompletion, isAbortError } from './chat';
import { ApiError } from './client';
import type { ChatCompletionRequest } from './types';

const req: ChatCompletionRequest = {
	model: 'm1',
	messages: [{ role: 'user', content: 'hi' }],
	temperature: 0.7,
	max_tokens: 16,
	top_p: 0.9,
	top_k: 40,
	stream: true
};

function sseResponse(frames: string[]): Response {
	const encoder = new TextEncoder();
	const stream = new ReadableStream<Uint8Array>({
		start(controller) {
			for (const frame of frames) {
				controller.enqueue(encoder.encode(frame));
			}
			controller.close();
		}
	});
	return new Response(stream, {
		status: 200,
		headers: { 'Content-Type': 'text/event-stream' }
	});
}

afterEach(() => {
	vi.restoreAllMocks();
});

describe('streamChatCompletion', () => {
	it('parses delta frames and stops on [DONE]', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(
			sseResponse([
				'data: {"choices":[{"delta":{"content":"hi"}}]}\n\n',
				'data: {"choices":[{"delta":{"content":" there"}}]}\n\n',
				'data: [DONE]\n\n',
				// Anything after [DONE] must be ignored.
				'data: {"choices":[{"delta":{"content":"!!"}}]}\n\n'
			])
		);

		const tokens: string[] = [];
		await streamChatCompletion(req, new AbortController().signal, (d) => tokens.push(d));

		expect(tokens).toEqual(['hi', ' there']);
	});

	it('ignores keep-alive comments and empty deltas', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(
			sseResponse([
				': keep-alive\n\n',
				'data: {"choices":[{"delta":{}}]}\n\n',
				'data: {"choices":[{"delta":{"content":"ok"}}]}\n\n',
				'data: [DONE]\n\n'
			])
		);

		const tokens: string[] = [];
		await streamChatCompletion(req, new AbortController().signal, (d) => tokens.push(d));

		expect(tokens).toEqual(['ok']);
	});

	it('throws an ApiError on a non-OK response', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(
			new Response('boom', { status: 500, statusText: 'Internal Server Error' })
		);

		await expect(
			streamChatCompletion(req, new AbortController().signal, () => {})
		).rejects.toBeInstanceOf(ApiError);
	});

	it('handles CRLF framing and flushes a final unterminated frame', async () => {
		vi.spyOn(globalThis, 'fetch').mockResolvedValue(
			sseResponse([
				'data: {"choices":[{"delta":{"content":"a"}}]}\r\n\r\n',
				// Final frame has no trailing blank line — must still be flushed.
				'data: {"choices":[{"delta":{"content":"b"}}]}'
			])
		);

		const tokens: string[] = [];
		await streamChatCompletion(req, new AbortController().signal, (d) => tokens.push(d));

		expect(tokens).toEqual(['a', 'b']);
	});
});

describe('isAbortError', () => {
	it('is true for user-cancel (AbortError) and timeout (TimeoutError)', () => {
		expect(isAbortError(new DOMException('cancelled', 'AbortError'))).toBe(true);
		// AbortSignal.timeout() raises TimeoutError, which must also be graceful.
		expect(isAbortError(new DOMException('timed out', 'TimeoutError'))).toBe(true);
	});

	it('is false for real errors', () => {
		expect(isAbortError(new Error('boom'))).toBe(false);
		expect(isAbortError(new ApiError(500, 'server', true))).toBe(false);
		expect(isAbortError(null)).toBe(false);
	});
});
