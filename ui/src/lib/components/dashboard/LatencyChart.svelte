<script lang="ts">
	import EChartsWrapper, { type DataSeries } from '../shared/EChartsWrapper.svelte';

	export interface LatencyPercentilePoint {
		timestamp: number;
		p50: number;
		p90: number;
		p99: number;
	}

	let { data }: { data: LatencyPercentilePoint[] } = $props();

	let series: DataSeries[] = $derived([
		{
			name: 'p50',
			data: data.map((d) => ({ timestamp: d.timestamp, value: d.p50 })),
			color: '#22c55e' // Green
		},
		{
			name: 'p90',
			data: data.map((d) => ({ timestamp: d.timestamp, value: d.p90 })),
			color: '#f59e0b' // Yellow/Orange
		},
		{
			name: 'p99',
			data: data.map((d) => ({ timestamp: d.timestamp, value: d.p99 })),
			color: '#ef4444' // Red
		}
	]);
</script>

<EChartsWrapper title="Latency Percentiles" yAxisLabel="ms" {series} height="100%" />
