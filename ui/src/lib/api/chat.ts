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
 * {@link ApiError} on a non-OK response, and propagates the `AbortError` from
 * the signal on cancel/timeout.
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

	for (;;) {
		const { value, done } = await reader.read();
		if (done) break;
		buffer += decoder.decode(value, { stream: true });

		// SSE frames are separated by a blank line; keep the trailing partial.
		const frames = buffer.split('\n\n');
		buffer = frames.pop() ?? '';

		for (const frame of frames) {
			const line = frame.trim();
			if (!line.startsWith('data:')) continue;
			const data = line.slice('data:'.length).trim();
			if (data === '[DONE]') return;
			try {
				const parsed = JSON.parse(data);
				const delta = parsed?.choices?.[0]?.delta?.content;
				if (typeof delta === 'string' && delta.length > 0) {
					onToken(delta);
				}
			} catch {
				// Ignore keep-alive comments and partial/non-JSON frames.
			}
		}
	}
}
