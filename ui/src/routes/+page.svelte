<script lang="ts">
	import {
		createServersQuery,
		createQueueQuery,
		createRestartServerMutation,
		createStopServerMutation,
		createStartServerMutation,
		prefetchServerDetail,
		createAggregatedMetricsQuery
	} from '$lib/api/queries';
	import { getAvailableActions, computeFleetHealth, type DetailServerAction } from '$lib/api/types';
	import { dashboardStore } from '$lib/stores/dashboard.svelte';
	import ServerGrid from '$lib/components/dashboard/ServerGrid.svelte';
	import ServerCard from '$lib/components/dashboard/ServerCard.svelte';
	import QueueDepthGauge from '$lib/components/dashboard/QueueDepthGauge.svelte';
	import SystemHealthBanner from '$lib/components/dashboard/SystemHealthBanner.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';
	import ThroughputChart from '$lib/components/dashboard/ThroughputChart.svelte';
	import LatencyChart from '$lib/components/dashboard/LatencyChart.svelte';
	import type { TimeSeriesPoint, LatencyPercentilePoint } from '$lib/api/types';
	import { formatTokensPerSecond } from '$lib/utils/format';
	import { goto } from '$app/navigation';
	import { toastStore } from '$lib/stores/toast.svelte';
	import { untrack } from 'svelte';

	const MAX_HISTORY_POINTS = 500;

	const serversQuery = createServersQuery();
	const queueQuery = createQueueQuery();
	const metricsQuery = createAggregatedMetricsQuery({});

	const restartMutation = createRestartServerMutation();
	const stopMutation = createStopServerMutation();
	const startMutation = createStartServerMutation();

	let servers = $derived(serversQuery.data ?? []);

	let healthData = $derived(computeFleetHealth(servers));

	let throughputHistory = $state<TimeSeriesPoint[]>([]);
	let latencyHistory = $state<LatencyPercentilePoint[]>([]);
	let lastMetricsTime = 0;

	$effect(() => {
		const metrics = metricsQuery.data as Record<string, unknown> | undefined;
		if (metrics && metrics.throughput_total !== undefined) {
			const now = Date.now();
			if (now - lastMetricsTime > 100) {
				// Throttle and prevent infinite loops
				lastMetricsTime = now;
				untrack(() => {
					let newThroughput = [
						...throughputHistory,
						{ timestamp: now, value: Number(metrics.throughput_total) }
					];
					if (newThroughput.length > MAX_HISTORY_POINTS)
						newThroughput = newThroughput.slice(-MAX_HISTORY_POINTS);
					throughputHistory = newThroughput;

					const latency = metrics.latency as Record<string, number> | undefined;
					if (latency) {
						let newLatency = [
							...latencyHistory,
							{
								timestamp: now,
								p50: latency.p50 || 0,
								p90: latency.p90 || 0,
								p99: latency.p99 || 0
							}
						];
						if (newLatency.length > MAX_HISTORY_POINTS)
							newLatency = newLatency.slice(-MAX_HISTORY_POINTS);
						latencyHistory = newLatency;
					}
				});
			}
		}
	});

	function handleAction(id: string, action: DetailServerAction) {
		if (action === 'restart') restartMutation.mutate(id);
		else if (action === 'stop') stopMutation.mutate(id);
		else if (action === 'start') startMutation.mutate(id);
	}

	$effect(() => {
		dashboardStore.announceSummary = () => {
			const totalToks = metricsQuery.data ? metricsQuery.data.throughput_total : 0;
			const qDepth = queueQuery.data ? queueQuery.data.length : 0;
			const msg = `${healthData.counts.healthy} servers healthy, ${healthData.counts.degraded} degraded. Total throughput ${formatTokensPerSecond(totalToks)}. Queue depth ${qDepth}.`;
			toastStore.add('info', msg);
		};
		return () => {
			dashboardStore.announceSummary = null;
		};
	});
</script>

<svelte:head>
	<title>Dashboard | LlamaForge</title>
</svelte:head>

<div class="mx-auto max-w-[1400px] space-y-6">
	<h1 tabindex="-1" class="sr-only">Dashboard</h1>

	<SystemHealthBanner healthStatus={healthData.status} serverCounts={healthData.counts} />

	<div class="grid grid-cols-1 gap-6 md:grid-cols-2">
		<div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6">
			<h2 class="mb-4 text-lg font-bold text-[var(--color-text-primary)]">Total Throughput</h2>
			{#if metricsQuery.isPending}
				<Skeleton width="100%" height="2rem" />
			{:else if metricsQuery.isError}
				<div class="text-sm text-[var(--color-status-failed)]">Failed to load throughput</div>
			{:else}
				<div class="text-3xl font-medium">
					{formatTokensPerSecond(metricsQuery.data?.throughput_total || 0)}
				</div>
			{/if}
		</div>
		<div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6">
			<h2 class="mb-4 text-lg font-bold text-[var(--color-text-primary)]">Queue Depth</h2>
			{#if queueQuery.isPending}
				<Skeleton width="100%" height="2rem" />
			{:else if queueQuery.isError}
				<div class="text-sm text-[var(--color-status-failed)]">Failed to load queue</div>
			{:else}
				<QueueDepthGauge depth={queueQuery.data?.length || 0} capacity={100} />
			{/if}
		</div>
	</div>

	<div class="mt-8 mb-4 flex items-center justify-between">
		<h2 class="text-xl font-bold">Servers</h2>
	</div>

	{#if serversQuery.isPending}
		<div
			class="grid w-full gap-6"
			style="grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));"
		>
			<Skeleton height="var(--card-min-height)" class="w-full" />
			<Skeleton height="var(--card-min-height)" class="w-full" />
			<Skeleton height="var(--card-min-height)" class="w-full" />
		</div>
	{:else if serversQuery.isError}
		<EmptyState
			title="Failed to load servers"
			description="There was an error communicating with the backend API."
			icon="✕"
			action={{ label: 'Retry', onClick: () => serversQuery.refetch() }}
		/>
	{:else if servers.length === 0}
		<EmptyState
			title="No servers configured"
			description="You haven't configured any LlamaForge servers yet."
			icon="🖥️"
			action={{ label: 'Configure Servers', onClick: () => goto('/config/servers') }}
		/>
	{:else}
		<ServerGrid itemCount={servers.length}>
			{#snippet children(focusedIndex: number)}
				{#each servers as server, i (server.id)}
					<ServerCard
						{server}
						isFocused={i === focusedIndex}
						availableActions={getAvailableActions(server.status)}
						onAction={handleAction}
						onHover={() => prefetchServerDetail(server.id)}
					/>
				{/each}
			{/snippet}
		</ServerGrid>
	{/if}

	<div class="mt-8 mb-4 flex items-center justify-between">
		<h2 class="text-xl font-bold">Metrics History</h2>
	</div>

	<div class="grid grid-cols-1 gap-6 lg:grid-cols-2">
		<div
			class="h-[400px] rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6"
		>
			<ThroughputChart data={throughputHistory} />
		</div>
		<div
			class="h-[400px] rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6"
		>
			<LatencyChart data={latencyHistory} />
		</div>
	</div>
</div>
