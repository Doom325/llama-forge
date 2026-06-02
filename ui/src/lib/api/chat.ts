import { ApiError } from './client';
import type { ChatCompletionRequest } from './types';

/**
 * Streams an OpenAI-compatible chat completion from `POST /v1/chat/completions`.
 *
 * Reads the `text/event-stream` response body incrementally and invokes
 * `onToken` for each content delta. The caller owns the {@link AbortSignal}
 * (typically `AbortSignal.any([controller.signal, AbortSignal.timeout(...)])`),
 * so user-cancel and the generation timeout both abort the underlying fetch.
 *
 * Resolves when the stream ends (`data: [DONE]`); rejects with an
 * {@link ApiError} on a non-OK response, and propagates the signal's abort
 * reason (`AbortError` on user cancel, `TimeoutError` on timeout).
 */
export async function streamChatCompletion(
	body: ChatCompletionRequest,
	signal: AbortSignal,
	onToken: (delta: string) => void
): Promise<void> {
	const response = await fetch('/v1/chat/completions', {
		method: 'POST',
		headers: {
			'Content-Type': 'application/json',
			Accept: 'text/event-stream',
			// CSRF guard — same custom header apiFetch attaches to mutations.
			'X-LlamaForge-Request': '1'
		},
		body: JSON.stringify(body),
		signal
	});

	if (!response.ok || !response.body) {
		throw new ApiError(
			response.status,
			`HTTP ${response.status} ${response.statusText}`,
			response.status >= 500
		);
	}

	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';

	// Handle one SSE frame. Returns true when the [DONE] sentinel is seen.
	const handleFrame = (frame: string): boolean => {
		const line = frame.trim();
		if (!line.startsWith('data:')) return false;
		const data = line.slice('data:'.length).trim();
		if (data === '[DONE]') return true;
		try {
			const parsed = JSON.parse(data);
			const delta = parsed?.choices?.[0]?.delta?.content;
			if (typeof delta === 'string' && delta.length > 0) {
				onToken(delta);
			}
		} catch {
			// Ignore keep-alive comments and partial/non-JSON frames.
		}
		return false;
	};

	for (;;) {
		const { value, done } = await reader.read();
		if (done) break;
		buffer += decoder.decode(value, { stream: true });

		// Frames are separated by a blank line; tolerate LF and CRLF framing and
		// keep the trailing partial in the buffer.
		const frames = buffer.split(/\r?\n\r?\n/);
		buffer = frames.pop() ?? '';
		for (const frame of frames) {
			if (handleFrame(frame)) return;
		}
	}

	// Flush a final frame that was not terminated by a blank line.
	if (buffer.trim()) handleFrame(buffer);
}

/**
 * True for the DOMExceptions that represent a graceful stop rather than a
 * failure: user-cancel raises `AbortError`, and `AbortSignal.timeout()` raises
 * `TimeoutError` (propagated as-is by `AbortSignal.any`). Both should end the
 * stream quietly instead of surfacing an error state.
 */
export function isAbortError(err: unknown): boolean {
	return err instanceof DOMException && (err.name === 'AbortError' || err.name === 'TimeoutError');
}
