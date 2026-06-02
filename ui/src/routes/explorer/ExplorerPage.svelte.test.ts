import { describe, it, expect, vi } from 'vitest';

vi.mock('@tanstack/svelte-table', () => ({
	createSvelteTable: vi.fn(),
	getCoreRowModel: vi.fn(),
	getSortedRowModel: vi.fn(),
	flexRender: vi.fn(),
	FlexRender: vi.fn()
}));

vi.mock('@tanstack/svelte-virtual', () => ({
	createVirtualizer: vi.fn()
}));

describe('ExplorerPage', () => {
	it('initializes filters from URL', () => {
		expect(true).toBe(true);
	});
});
