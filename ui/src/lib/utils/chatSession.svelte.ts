import { streamChatCompletion, isAbortError } from '$lib/api/chat';
import { ApiError } from '$lib/api/client';
import { TIMEOUTS } from '$lib/utils/constants';
import { toastStore } from '$lib/stores/toast.svelte';
import type {
	ChatCompletionRequest,
	GenerationParams,
	Message,
	ModelInfo,
	ResponseMetrics,
	StreamingState
} from '$lib/api/types';

// Rough token estimate (~4 chars/token). The backend is authoritative, but
// usage stats are not always returned mid-stream, so we approximate.
function estimateTokens(text: string): number {
	return Math.max(1, Math.ceil(text.length / 4));
}

export interface ChatSession {
	readonly messages: Message[];
	readonly streamingState: StreamingState;
	/** Polite screen-reader announcement for the latest lifecycle transition. */
	readonly liveMessage: string;
	send(content: string, model: ModelInfo, params: GenerationParams): Promise<void>;
	cancel(): void;
	clear(): void;
}

/**
 * Reactive single-conversation chat session: owns the message list, the
 * {@link StreamingState} machine, and the per-request `AbortController`.
 * Shared by the primary playground pane and each comparison pane so the
 * streaming logic lives in exactly one place. Must be called during component
 * initialisation.
 */
export function createChatSession(): ChatSession {
	let messages = $state<Message[]>([]);
	let streamingState = $state<StreamingState>({ status: 'idle' });
	let controller = $state<AbortController | null>(null);
	let liveMessage = $state('');

	async function send(content: string, model: ModelInfo, params: GenerationParams): Promise<void> {
		messages = [...messages, { role: 'user', content, timestamp: Date.now() }];

		// Go straight to streaming. The `queued` state + its announcement are wired
		// for when the backend reports a real queue position; we don't fake one.
		streamingState = { status: 'streaming', tokens: '', ttft: 0 };

		controller = new AbortController();
		const req: ChatCompletionRequest = {
			model: model.id,
			messages: messages.map((m) => ({ role: m.role, content: m.content })),
			temperature: params.temperature,
			max_tokens: params.max_tokens,
			top_p: params.top_p,
			top_k: params.top_k,
			stream: true
		};

		const start = Date.now();
		let ttft = 0;
		let tokens = '';

		try {
			await streamChatCompletion(
				req,
				AbortSignal.any([controller.signal, AbortSignal.timeout(TIMEOUTS.PLAYGROUND_ABORT)]),
				(delta) => {
					if (ttft === 0) ttft = Date.now() - start;
					tokens += delta;
					streamingState = { status: 'streaming', tokens, ttft };
				}
			);

			const elapsedMs = Date.now() - start;
			const totalTokens = estimateTokens(tokens);
			const tokensPerSecond = elapsedMs > 0 ? totalTokens / (elapsedMs / 1000) : 0;
			const metrics: ResponseMetrics = { ttft, tokensPerSecond, totalTokens };

			messages = [
				...messages,
				{ role: 'assistant', content: tokens, timestamp: Date.now(), metrics }
			];
			streamingState = { status: 'complete', response: tokens, metrics };
			liveMessage = 'Generation complete.';
			streamingState = { status: 'idle' };
		} catch (err) {
			if (isAbortError(err)) {
				// Graceful stop. AbortSignal.any propagates the originating signal's
				// reason, so a 120s timeout surfaces as TimeoutError (not AbortError);
				// controller.signal.aborted distinguishes user-cancel from timeout.
				const userCancelled = controller?.signal.aborted ?? false;
				if (tokens.length > 0) {
					messages = [...messages, { role: 'assistant', content: tokens, timestamp: Date.now() }];
				}
				streamingState = { status: 'idle' };
				liveMessage = userCancelled ? 'Generation stopped.' : 'Generation timed out.';
			} else {
				const message =
					err instanceof ApiError || err instanceof Error ? err.message : 'Generation failed.';
				streamingState = {
					status: 'error',
					error: { message, retryable: true },
					partialResponse: tokens.length > 0 ? tokens : null
				};
				liveMessage = 'Generation failed.';
				toastStore.add('error', message);
			}
		} finally {
			controller = null;
		}
	}

	function cancel(): void {
		controller?.abort();
	}

	function clear(): void {
		messages = [];
		streamingState = { status: 'idle' };
		liveMessage = '';
	}

	return {
		get messages() {
			return messages;
		},
		get streamingState() {
			return streamingState;
		},
		get liveMessage() {
			return liveMessage;
		},
		send,
		cancel,
		clear
	};
}
