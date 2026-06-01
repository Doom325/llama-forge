import { describe, it, expect, vi, beforeEach, afterEach, type Mock } from 'vitest';
import { apiFetch } from './client';

describe('Api Client', () => {
	let fetchMock: Mock;

	beforeEach(() => {
		fetchMock = vi.fn();
		global.fetch = fetchMock;
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('adds CSRF header to mutations but not GET', async () => {
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: new Headers(),
			json: async () => ({})
		});

		await apiFetch('/api/test', { method: 'POST' });

		expect(fetchMock).toHaveBeenCalledTimes(1);
		const postHeaders = fetchMock.mock.calls[0]?.[1]?.headers as Headers;
		expect(postHeaders.get('X-LlamaForge-Request')).toBe('1');

		fetchMock.mockClear();
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: new Headers(),
			json: async () => ({})
		});

		await apiFetch('/api/test', { method: 'GET' });
		const getHeaders = fetchMock.mock.calls[0]?.[1]?.headers as Headers;
		expect(getHeaders.get('X-LlamaForge-Request')).toBeNull();
	});

	it('normalizes errors correctly', async () => {
		// 404
		fetchMock.mockResolvedValueOnce({
			ok: false,
			status: 404,
			statusText: 'Not Found',
			headers: new Headers(),
			json: async () => ({ message: 'Not found here' })
		});

		await expect(apiFetch('/api/test')).rejects.toMatchObject({
			status: 404,
			message: 'Not found here',
			retryable: false
		});

		// 500
		fetchMock.mockResolvedValueOnce({
			ok: false,
			status: 500,
			statusText: 'Server Error',
			headers: new Headers(),
			json: async () => ({ message: 'Boom' })
		});

		await expect(apiFetch('/api/test')).rejects.toMatchObject({
			status: 500,
			retryable: true
		});

		// Network Error
		fetchMock.mockRejectedValueOnce(new Error('Failed to fetch'));

		await expect(apiFetch('/api/test')).rejects.toMatchObject({
			status: 0,
			retryable: true,
			message: 'Failed to fetch'
		});

		// Timeout
		const timeoutError = new DOMException('Timeout', 'TimeoutError');
		fetchMock.mockRejectedValueOnce(timeoutError);

		await expect(apiFetch('/api/test')).rejects.toMatchObject({
			status: 408,
			retryable: true,
			message: 'Request timed out'
		});
	});
});
