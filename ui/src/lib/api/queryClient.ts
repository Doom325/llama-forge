import { QueryClient } from '@tanstack/svelte-query';
import { LiveDataStore } from '$lib/stores/liveData.svelte';

export const queryClient = new QueryClient({
	defaultOptions: {
		queries: {
			staleTime: 5_000,
			gcTime: 300_000,
			refetchOnWindowFocus: true,
			retry: 3,
			retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30_000)
		},
		mutations: {
			retry: 0
		}
	}
});

export const liveDataStore = new LiveDataStore(queryClient);
