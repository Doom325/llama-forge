import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import DiffPreview from './DiffPreview.svelte';

describe('DiffPreview', () => {
	it('shows changed, added, and removed fields with their values', () => {
		render(DiffPreview, {
			original: { name: 'old', port: 8080, removed: 'gone' },
			modified: { name: 'new', port: 8080, added: 'fresh' },
			open: true,
			onClose: vi.fn()
		});

		expect(screen.getByText('Review Changes')).toBeInTheDocument();

		// changed: name
		expect(screen.getByText('name')).toBeInTheDocument();
		expect(screen.getByText('- "old"')).toBeInTheDocument();
		expect(screen.getByText('+ "new"')).toBeInTheDocument();

		// added: added
		expect(screen.getByText('added')).toBeInTheDocument();
		expect(screen.getByText('+ "fresh"')).toBeInTheDocument();

		// removed: removed
		expect(screen.getByText('removed')).toBeInTheDocument();
		expect(screen.getByText('- "gone"')).toBeInTheDocument();

		// unchanged port must not appear
		expect(screen.queryByText('port')).not.toBeInTheDocument();
	});

	it('shows "No changes." when objects are equal', () => {
		render(DiffPreview, {
			original: { name: 'same', port: 8080 },
			modified: { name: 'same', port: 8080 },
			open: true,
			onClose: vi.fn()
		});

		expect(screen.getByText('No changes.')).toBeInTheDocument();
	});
});
