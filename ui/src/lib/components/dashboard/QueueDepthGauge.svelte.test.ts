import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import QueueDepthGauge from './QueueDepthGauge.svelte';

describe('QueueDepthGauge', () => {
	it('renders correctly with ARIA attributes and hidden fallback', () => {
		render(QueueDepthGauge, { depth: 5, capacity: 10 });

		const meter = screen.getByRole('meter');
		expect(meter).toBeInTheDocument();
		expect(meter).toHaveAttribute('aria-valuenow', '5');
		expect(meter).toHaveAttribute('aria-valuemin', '0');
		expect(meter).toHaveAttribute('aria-valuemax', '10');
		expect(meter).toHaveAttribute('aria-label', 'Queue depth: 5 of 10');
	});
});
