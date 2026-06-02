<script lang="ts">
	import EChartsWrapper, { type DataSeries } from '../shared/EChartsWrapper.svelte';
	import type { LatencyPercentilePoint } from '$lib/api/types';

	let { data }: { data: LatencyPercentilePoint[] } = $props();

	// Colorblind-safe palette (Okabe-Ito inspired)
	let series: DataSeries[] = $derived([
		{
			name: 'p50',
			data: data.map((d) => ({ timestamp: d.timestamp, value: d.p50 })),
			color: '#56B4E9', // Sky Blue
			type: 'dashed'
		},
		{
			name: 'p90',
			data: data.map((d) => ({ timestamp: d.timestamp, value: d.p90 })),
			color: '#E69F00', // Orange
			type: 'dotted'
		},
		{
			name: 'p99',
			data: data.map((d) => ({ timestamp: d.timestamp, value: d.p99 })),
			color: '#D55E00', // Vermilion
			type: 'solid'
		}
	]);
</script>

<EChartsWrapper title="Latency Percentiles" yAxisLabel="ms" {series} height="100%" />
