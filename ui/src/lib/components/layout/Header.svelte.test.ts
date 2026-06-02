import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import HeaderWrapper from '../testing/HeaderWrapper.svelte';
import { liveDataStore } from '$lib/api/queryClient';

// Mock app modules
vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/')
	}
}));

describe('Header', () => {
	it('renders header elements, breadcrumbs and child components', () => {
		liveDataStore.connected = true;

		render(HeaderWrapper);

		expect(screen.getByRole('banner')).toBeInTheDocument(); // <header> implicit role
		expect(screen.getByLabelText('Collapse sidebar')).toBeInTheDocument();
		expect(screen.getByLabelText('Toggle theme')).toBeInTheDocument();
		expect(screen.getByLabelText('Keyboard shortcuts')).toBeInTheDocument();

		// By default testing env runs at / (Dashboard)
		expect(screen.getByText('Dashboard', { selector: 'div' })).toBeInTheDocument();
		expect(screen.getByText('Connected')).toBeInTheDocument();
	});
});
