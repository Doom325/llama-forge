import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import ConnectionStatus from './ConnectionStatus.svelte';
import { liveDataStore } from '$lib/api/queryClient';
import { flushSync } from 'svelte';

describe('ConnectionStatus', () => {
	it('shows reconnecting badge when not connected', () => {
		liveDataStore.connected = false;
		render(ConnectionStatus);
		flushSync();
		expect(screen.getByText('Reconnecting')).toBeInTheDocument();
	});

	it('shows sr-only connected text when connected', () => {
		liveDataStore.connected = true;
		render(ConnectionStatus);
		flushSync();
		const el = screen.getByText('Connected');
		expect(el).toBeInTheDocument();
		expect(el).toHaveClass('sr-only');
	});
});
