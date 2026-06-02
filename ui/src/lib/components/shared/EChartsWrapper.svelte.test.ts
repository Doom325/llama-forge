import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import EChartsWrapper from './EChartsWrapper.svelte';
import { tick } from 'svelte';

const setOptionMock = vi.fn();
const resizeMock = vi.fn();
const disposeMock = vi.fn();
const initMock = vi.fn().mockReturnValue({
	setOption: setOptionMock,
	resize: resizeMock,
	dispose: disposeMock
});

vi.mock('echarts', () => {
	return {
		init: initMock
	};
});

describe('EChartsWrapper', () => {
	beforeEach(() => {
		setOptionMock.mockClear();
		resizeMock.mockClear();
		disposeMock.mockClear();
		initMock.mockClear();

		global.ResizeObserver = class ResizeObserver {
			observe() {}
			unobserve() {}
			disconnect() {}
		};

		vi.useFakeTimers();
	});
	it('loads lazily and sets up sliding window properly', async () => {
		const seriesData = {
			name: 'test',
			data: Array.from({ length: 600 }).map((_, i) => ({ timestamp: i * 1000, value: i }))
		};

		const { unmount, container } = render(EChartsWrapper, {
			series: [seriesData],
			title: 'Test',
			yAxisLabel: 'y',
			maxPoints: 500
		});

		// By default skeleton is there since it's lazy
		expect(container.querySelector('.animate-pulse')).toBeInTheDocument(); // Skeleton class

		// Allow dynamic import to resolve
		await tick();
		await vi.runAllTimersAsync();

		expect(initMock).toHaveBeenCalled();
		expect(setOptionMock).toHaveBeenCalled();

		// Check sliding window
		const callArgs = setOptionMock.mock.calls[0]?.[0] as Record<string, unknown>;
		const firstSeries = (callArgs.series as Array<Record<string, unknown>>)[0];
		const firstSeriesData = firstSeries?.data as Array<number[]>;

		expect(firstSeriesData?.length).toBe(500);
		expect(firstSeriesData?.[0]?.[1]).toBe(100); // 600 - 500 = 100th element
		unmount();
		expect(disposeMock).toHaveBeenCalled();
	});

	it('renders accessible table correctly', async () => {
		const seriesData = {
			name: 'Metric',
			data: Array.from({ length: 50 }).map((_, i) => ({ timestamp: 100000 + i * 1000, value: i }))
		};

		render(EChartsWrapper, {
			series: [seriesData],
			title: 'Test Chart',
			yAxisLabel: 'val'
		});

		await tick();

		const table = screen.getByRole('table', { hidden: true });
		expect(table).toBeInTheDocument();
		expect(
			screen.getByText(
				'Showing most recent 20 of 50 data points. Full data available via CSV export.',
				{ selector: 'p' }
			)
		).toBeInTheDocument();

		// 1 header row + 20 data rows
		const rows = screen.getAllByRole('row', { hidden: true });
		expect(rows.length).toBe(21);
	});
});
