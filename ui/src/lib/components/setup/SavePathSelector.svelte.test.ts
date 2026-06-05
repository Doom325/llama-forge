import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import SavePathSelector from './SavePathSelector.svelte';

describe('SavePathSelector', () => {
	it('emits path edits', async () => {
		const onpathchange = vi.fn();
		render(SavePathSelector, {
			path: '/x',
			force: false,
			onpathchange,
			onforcechange: vi.fn()
		});
		await fireEvent.input(screen.getByLabelText('Save configuration to'), {
			target: { value: '/y/config.toml' }
		});
		expect(onpathchange).toHaveBeenCalledWith('/y/config.toml');
	});

	it('emits the overwrite toggle', async () => {
		const onforcechange = vi.fn();
		render(SavePathSelector, {
			path: '/x',
			force: false,
			onpathchange: vi.fn(),
			onforcechange
		});
		await fireEvent.click(screen.getByLabelText(/Overwrite/));
		expect(onforcechange).toHaveBeenCalledWith(true);
	});
});
