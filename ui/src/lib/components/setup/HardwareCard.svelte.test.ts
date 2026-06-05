import { render, screen } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import HardwareCard from './HardwareCard.svelte';

describe('HardwareCard', () => {
	it('maps detected → Healthy badge', () => {
		render(HardwareCard, { type: 'gpu', label: 'RTX 4090', details: '24 GB', status: 'detected' });
		expect(screen.getByText('RTX 4090')).toBeInTheDocument();
		expect(screen.getByText('24 GB')).toBeInTheDocument();
		expect(screen.getByText('Healthy')).toBeInTheDocument();
	});

	it('maps error → Failed badge', () => {
		render(HardwareCard, {
			type: 'cpu',
			label: 'CPU',
			details: 'detection error',
			status: 'error'
		});
		expect(screen.getByText('Failed')).toBeInTheDocument();
	});

	it('maps not_detected → Degraded badge', () => {
		render(HardwareCard, {
			type: 'gpu',
			label: 'GPU',
			details: 'none found',
			status: 'not_detected'
		});
		expect(screen.getByText('Degraded')).toBeInTheDocument();
	});
});
