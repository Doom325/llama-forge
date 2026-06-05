import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ModelTable from './ModelTable.svelte';
import type { DiscoveredModel } from '$lib/api/types';

const models: DiscoveredModel[] = [
	{
		id: 'm1',
		path: '/a.gguf',
		filename: 'a.gguf',
		size_bytes: 1024 ** 3,
		quantization_hint: 'Q4_K_M',
		architecture_hint: 'llama'
	},
	{
		id: 'm2',
		path: '/b.gguf',
		filename: 'b.gguf',
		size_bytes: 2048,
		quantization_hint: 'Q8_0',
		architecture_hint: 'mistral'
	}
];

describe('ModelTable', () => {
	it('renders a row per model and toggles a single selection', async () => {
		const ontoggle = vi.fn();
		render(ModelTable, { models, selectedIds: new Set<string>(), ontoggle, ontoggleAll: vi.fn() });
		expect(screen.getByText('a.gguf')).toBeInTheDocument();
		expect(screen.getByText('b.gguf')).toBeInTheDocument();
		await fireEvent.click(screen.getByLabelText('Select a.gguf'));
		expect(ontoggle).toHaveBeenCalledWith('m1');
	});

	it('reflects + toggles select-all', async () => {
		const ontoggleAll = vi.fn();
		render(ModelTable, {
			models,
			selectedIds: new Set(['m1', 'm2']),
			ontoggle: vi.fn(),
			ontoggleAll
		});
		const all = screen.getByLabelText('Select all models') as HTMLInputElement;
		expect(all.checked).toBe(true);
		await fireEvent.click(all);
		expect(ontoggleAll).toHaveBeenCalledWith(false);
	});
});
