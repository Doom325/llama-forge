<script lang="ts">
	import EChartsWrapper, { type DataSeries } from '../shared/EChartsWrapper.svelte';
	import type { TimeSeriesPoint } from '$lib/api/types';

	let { data }: { data: TimeSeriesPoint[] } = $props();

	let series: DataSeries[] = $derived([
		{
			name: 'Throughput',
			data,
			color: '#3b82f6' // color-status-pending-dark or another safe blue
		}
	]);
</script>

<EChartsWrapper title="Total Throughput" yAxisLabel="tokens / sec" {series} height="100%" />
