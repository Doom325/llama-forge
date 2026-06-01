import { TIMEOUTS } from '$lib/utils/constants';

export class ApiError extends Error {
	constructor(
		public status: number,
		message: string,
		public retryable: boolean
	) {
		super(message);
		this.name = 'ApiError';
	}
}

interface FetchOptions extends RequestInit {
	timeout?: number;
}

export async function apiFetch<T>(url: string, options: FetchOptions = {}): Promise<T> {
	const { timeout, ...fetchOptions } = options;
	const isMutation =
		fetchOptions.method && fetchOptions.method !== 'GET' && fetchOptions.method !== 'HEAD';

	const defaultTimeout = isMutation ? TIMEOUTS.MUTATION : TIMEOUTS.GET;
	const actualTimeout = timeout ?? defaultTimeout;

	const headers = new Headers(fetchOptions.headers);
	if (isMutation) {
		headers.set('X-LlamaForge-Request', '1');
	}
	if (!headers.has('Accept')) {
		headers.set('Accept', 'application/json');
	}
	if (isMutation && !headers.has('Content-Type') && typeof fetchOptions.body === 'string') {
		headers.set('Content-Type', 'application/json');
	}

	try {
		const response = await fetch(url, {
			...fetchOptions,
			headers,
			signal: AbortSignal.timeout(actualTimeout)
		});

		if (!response.ok) {
			let errorMessage = `HTTP ${response.status} ${response.statusText}`;
			try {
				const errorData = await response.json();
				if (errorData && typeof errorData.message === 'string') {
					errorMessage = errorData.message;
				}
			} catch {
				// Ignore JSON parse errors for error responses
			}

			// 4xx non-retryable, 5xx retryable
			const retryable = response.status >= 500;
			throw new ApiError(response.status, errorMessage, retryable);
		}

		// Handle empty responses
		if (response.status === 204 || response.headers.get('content-length') === '0') {
			return {} as T;
		}

		return (await response.json()) as T;
	} catch (error) {
		if (error instanceof ApiError) {
			throw error;
		}

		if (error instanceof DOMException && error.name === 'TimeoutError') {
			throw new ApiError(408, 'Request timed out', true);
		}

		// Network error
		throw new ApiError(0, error instanceof Error ? error.message : 'Network error', true);
	}
}
