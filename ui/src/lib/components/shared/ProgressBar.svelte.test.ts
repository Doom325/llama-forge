import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import ProgressBar from './ProgressBar.svelte';

describe('ProgressBar', () => {
	it('renders determinate with aria-valuenow + percentage label', () => {
		render(ProgressBar, { value: 42, label: 'Building' });
		const bar = screen.getByRole('progressbar');
		expect(bar).toHaveAttribute('aria-valuenow', '42');
		expect(bar).toHaveAttribute('aria-valuemin', '0');
		expect(bar).toHaveAttribute('aria-valuemax', '100');
		expect(screen.getByText('42%')).toBeInTheDocument();
	});

	it('clamps out-of-range values to [0, 100]', () => {
		render(ProgressBar, { value: 150 });
		expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '100');
	});

	it('is indeterminate (no aria-valuenow) when value is null', () => {
		render(ProgressBar, { value: null, label: 'Working' });
		const bar = screen.getByRole('progressbar');
		expect(bar).not.toHaveAttribute('aria-valuenow');
		expect(bar).toHaveAttribute('aria-label', 'Working');
	});
});
