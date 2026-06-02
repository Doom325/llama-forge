<script lang="ts">
	import { onMount } from 'svelte';
	import Skeleton from './Skeleton.svelte';
	import type { EChartsType, EChartsOption } from 'echarts';

	export interface TimeSeriesPoint {
		timestamp: number;
		value: number;
	}

	export interface DataSeries {
		name: string;
		data: TimeSeriesPoint[];
		color?: string;
	}

	let {
		series = [],
		title,
		yAxisLabel,
		maxPoints = 500,
		height = '100%'
	}: {
		series: DataSeries[];
		title: string;
		yAxisLabel: string;
		maxPoints?: number;
		height?: string;
	} = $props();

	let container: HTMLDivElement | null = null;
	let chartInstance: EChartsType | null = null;
	let loaded = $state(false);

	let rafId: number | null = null;
	let updatePending = false;

	// Keep a copy of data capped at maxPoints
	let windowedSeries = $state<DataSeries[]>([]);

	$effect(() => {
		// Window data
		windowedSeries = series.map((s) => {
			const data = s.data.length > maxPoints ? s.data.slice(-maxPoints) : s.data;
			return { ...s, data };
		});

		scheduleUpdate();
	});

	function scheduleUpdate() {
		if (typeof document !== 'undefined' && document.hidden) return; // Background tab pausing
		if (!chartInstance || updatePending) return;

		updatePending = true;
		rafId = requestAnimationFrame(() => {
			updatePending = false;
			rafId = null;
			applyOptions();
		});
	}

	function applyOptions() {
		if (!chartInstance) return;

		const echartSeries = windowedSeries.map((s) => ({
			name: s.name,
			type: 'line',
			showSymbol: false,
			data: s.data.map((p) => [p.timestamp, p.value]),
			itemStyle: s.color ? { color: s.color } : undefined,
			lineStyle: s.color ? { color: s.color } : undefined
		}));

		const option: EChartsOption = {
			tooltip: {
				trigger: 'axis',
				formatter: function (params: unknown) {
					// eslint-disable-next-line @typescript-eslint/no-explicit-any
					const pArr = params as any[];
					if (!pArr || pArr.length === 0) return '';
					const date = new Date(pArr[0].value[0]);
					let tooltipStr = `${date.toLocaleTimeString()}<br/>`;
					// eslint-disable-next-line @typescript-eslint/no-explicit-any
					pArr.forEach((param: any) => {
						const marker = param.marker;
						tooltipStr += `${marker} ${param.seriesName}: ${param.value[1]}<br/>`;
					});
					return tooltipStr;
				}
			},
			grid: {
				left: '3%',
				right: '4%',
				bottom: '3%',
				containLabel: true
			},
			xAxis: {
				type: 'time',
				splitLine: { show: false }
			},
			yAxis: {
				type: 'value',
				name: yAxisLabel
			},
			// eslint-disable-next-line @typescript-eslint/no-explicit-any
			series: echartSeries as any,
			animation: false // For performance on frequent updates
		};

		chartInstance.setOption(option);
	}

	onMount(() => {
		let resizeObserver: ResizeObserver | null = null;

		const handleVisibilityChange = () => {
			if (!document.hidden && windowedSeries.length > 0) {
				scheduleUpdate(); // Bulk update on foregrounding
			}
		};
		document.addEventListener('visibilitychange', handleVisibilityChange);

		import('echarts').then((echarts) => {
			if (!container) return; // unmounted before load

			// Try dark theme based on data attribute
			const isDark = document.documentElement.getAttribute('data-theme') === 'dark';
			chartInstance = echarts.init(container, isDark ? 'dark' : undefined);
			loaded = true;

			applyOptions();

			resizeObserver = new ResizeObserver(() => {
				chartInstance?.resize();
			});
			resizeObserver.observe(container);
		});

		return () => {
			document.removeEventListener('visibilitychange', handleVisibilityChange);
			if (rafId) cancelAnimationFrame(rafId);
			if (resizeObserver) resizeObserver.disconnect();
			if (chartInstance) {
				chartInstance.dispose();
				chartInstance = null;
			}
		};
	});

	// Data for accessible table (max 20 points)
	let accessibleTableData = $derived(
		windowedSeries.map((s) => ({
			name: s.name,
			data: s.data.slice(-20)
		}))
	);

	let maxPointsInSeries = $derived(
		windowedSeries.reduce((max, s) => Math.max(max, s.data.length), 0)
	);
</script>

<div
	role="img"
	aria-label="{title} chart"
	class="relative w-full overflow-hidden"
	style="height: {height};"
>
	{#if !loaded}
		<Skeleton width="100%" height="100%" class="absolute inset-0" />
	{/if}
	<div
		bind:this={container}
		class="absolute inset-0"
		aria-hidden="true"
		style="opacity: {loaded ? 1 : 0}"
	></div>

	<div class="sr-only">
		<p>
			Showing most recent {Math.min(20, maxPointsInSeries)} of {maxPointsInSeries} data points. Full data
			available via CSV export.
		</p>
		<table>
			<caption>{title} data</caption>
			<thead>
				<tr>
					<th scope="col">Time</th>
					{#each accessibleTableData as s (s.name)}
						<th scope="col">{s.name}</th>
					{/each}
				</tr>
			</thead>
			<tbody>
				{#if accessibleTableData.length > 0 && accessibleTableData[0] && accessibleTableData[0].data.length > 0}
					{#each accessibleTableData[0].data as point, i (i)}
						<tr>
							<td>{new Date(point.timestamp).toLocaleTimeString()}</td>
							{#each accessibleTableData as s (s.name)}
								<td>{s.data[i]?.value ?? ''}</td>
							{/each}
						</tr>
					{/each}
				{/if}
			</tbody>
		</table>
	</div>
</div>
