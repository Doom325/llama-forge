import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import userEvent from '@testing-library/user-event';
import ParameterSidebar from './ParameterSidebar.svelte';
import type { GenerationParams } from '$lib/api/types';

const params: GenerationParams = {
	temperature: 0.7,
	max_tokens: 2048,
	top_p: 0.9,
	top_k: 40
};
const defaults: GenerationParams = { ...params };

describe('ParameterSidebar', () => {
	it('calls onChange with the new temperature when the slider moves', async () => {
		const onChange = vi.fn();
		render(ParameterSidebar, { params, defaults, onChange, onReset: vi.fn() });

		const slider = screen.getByLabelText(/Temperature/);
		await fireEvent.input(slider, { target: { value: '1.5' } });

		expect(onChange).toHaveBeenCalledWith(
			expect.objectContaining({ temperature: 1.5, max_tokens: 2048 })
		);
	});

	it('calls onReset when the reset button is clicked', async () => {
		const user = userEvent.setup();
		const onReset = vi.fn();
		render(ParameterSidebar, { params, defaults, onChange: vi.fn(), onReset });

		await user.click(screen.getByRole('button', { name: 'Reset to defaults' }));
		expect(onReset).toHaveBeenCalledTimes(1);
	});
});
