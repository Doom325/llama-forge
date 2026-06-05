import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import StepNavigation from './StepNavigation.svelte';

describe('StepNavigation', () => {
	it('disables Next when canAdvance is false', () => {
		render(StepNavigation, { canGoBack: true, canAdvance: false });
		expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
	});

	it('enables Next when canAdvance and fires onnext', async () => {
		const onnext = vi.fn();
		render(StepNavigation, { canGoBack: true, canAdvance: true, onnext });
		const next = screen.getByRole('button', { name: 'Next' });
		expect(next).toBeEnabled();
		await fireEvent.click(next);
		expect(onnext).toHaveBeenCalledOnce();
	});

	it('disables Next while loading', () => {
		render(StepNavigation, { canGoBack: true, canAdvance: true, isLoading: true });
		expect(screen.getByRole('button', { name: 'Working…' })).toBeDisabled();
	});

	it('disables Back when canGoBack is false', () => {
		render(StepNavigation, { canGoBack: false, canAdvance: true });
		expect(screen.getByRole('button', { name: 'Back' })).toBeDisabled();
	});

	it('fires onback', async () => {
		const onback = vi.fn();
		render(StepNavigation, { canGoBack: true, canAdvance: false, onback });
		await fireEvent.click(screen.getByRole('button', { name: 'Back' }));
		expect(onback).toHaveBeenCalledOnce();
	});

	it('shows Skip only when canSkip and fires onskip', async () => {
		const onskip = vi.fn();
		render(StepNavigation, { canGoBack: true, canAdvance: false, canSkip: true, onskip });
		const skip = screen.getByRole('button', { name: 'Skip' });
		await fireEvent.click(skip);
		expect(onskip).toHaveBeenCalledOnce();
	});

	it('hides Skip by default', () => {
		render(StepNavigation, { canGoBack: true, canAdvance: true });
		expect(screen.queryByRole('button', { name: 'Skip' })).toBeNull();
	});
});
