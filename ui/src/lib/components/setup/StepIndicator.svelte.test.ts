import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import StepIndicator from './StepIndicator.svelte';
import type { IndicatorStep } from './StepIndicator.svelte';

const steps: IndicatorStep[] = [
	{ label: 'Welcome', status: 'complete' },
	{ label: 'Hardware', status: 'current' },
	{ label: 'Binary', status: 'upcoming' }
];

describe('StepIndicator', () => {
	it('marks the current step with aria-current="step"', () => {
		const { container } = render(StepIndicator, { steps });
		const current = container.querySelector('[aria-current="step"]');
		expect(current).not.toBeNull();
		expect(current?.textContent).toContain('Hardware');
	});

	it('renders completed steps as navigable buttons and emits onnavigate', async () => {
		const onnavigate = vi.fn();
		render(StepIndicator, { steps, onnavigate });
		const button = screen.getByRole('button', { name: /Welcome/ });
		await fireEvent.click(button);
		expect(onnavigate).toHaveBeenCalledWith(0);
	});

	it('does not render upcoming steps as buttons', () => {
		render(StepIndicator, { steps, onnavigate: vi.fn() });
		expect(screen.queryByRole('button', { name: /Binary/ })).toBeNull();
	});

	it('does not render buttons when onnavigate is absent', () => {
		render(StepIndicator, { steps });
		expect(screen.queryByRole('button')).toBeNull();
	});
});
