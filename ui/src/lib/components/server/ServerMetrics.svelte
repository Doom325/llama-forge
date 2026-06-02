<script lang="ts">
	import type {
		ServerMetricsSummary,
		TimeSeriesPoint,
		LatencyPercentilePoint
	} from '$lib/api/types';
	import ThroughputChart from '$lib/components/dashboard/ThroughputChart.svelte';
	import LatencyChart from '$lib/components/dashboard/LatencyChart.svelte';

	let {
		metrics,
		throughputHistory,
		latencyHistory
	}: {
		metrics: ServerMetricsSummary | undefined;
		throughputHistory: TimeSeriesPoint[];
		latencyHistory: LatencyPercentilePoint[];
	} = $props();
</script>

<div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6">
	<div class="mb-4 flex items-center justify-between">
		<h3 class="text-lg font-bold text-[var(--color-text-primary)]">Performance Metrics</h3>
		{#if metrics}
			<div class="flex items-center gap-4 text-sm">
				<div class="flex items-center gap-1.5">
					<span class="text-[var(--color-text-secondary)]">Error Rate:</span>
					<span
						class="font-medium {metrics.errorRate > 5
							? 'text-[var(--color-status-failed)]'
							: 'text-[var(--color-text-primary)]'}">{metrics.errorRate.toFixed(2)}%</span
					>
				</div>
			</div>
		{/if}
	</div>

	<div class="mb-8 grid grid-cols-1 gap-6 lg:grid-cols-2">
		<div class="h-[300px] w-full">
			<ThroughputChart data={throughputHistory} />
		</div>
		<div class="h-[300px] w-full">
			<LatencyChart data={latencyHistory} />
		</div>
	</div>
</div>
