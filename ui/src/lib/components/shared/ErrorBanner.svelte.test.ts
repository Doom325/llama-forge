import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import ErrorBanner from './ErrorBanner.svelte';

describe('ErrorBanner', () => {
	it('renders message', () => {
		render(ErrorBanner, { message: 'Test error', variant: 'error' });
		expect(screen.getByText('Test error')).toBeInTheDocument();
	});
});
