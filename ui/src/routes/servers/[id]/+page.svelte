<script lang="ts">
	import { page } from '$app/state';
	import {
		createServerDetailQuery,
		createServerMetricsQuery,
		createRestartServerMutation,
		createStopServerMutation,
		createStartServerMutation,
		createForceKillServerMutation
	} from '$lib/api/queries';
	import {
		getDetailAvailableActions,
		type DetailServerAction,
		type TimeSeriesPoint,
		type LatencyPercentilePoint
	} from '$lib/api/types';
	import { untrack } from 'svelte';
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
	import SlotGrid, { type SlotState } from '$lib/components/server/SlotGrid.svelte';
	import ServerMetrics from '$lib/components/server/ServerMetrics.svelte';
	import ServerActionBar from '$lib/components/server/ServerActionBar.svelte';
	import ConfirmDialog from '$lib/components/shared/ConfirmDialog.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import EmptyState from '$lib/components/shared/EmptyState.svelte';

	const serverId = page.params.id || '';

	const serverQuery = createServerDetailQuery(serverId);
	const metricsQuery = createServerMetricsQuery(serverId);

	const restartMutation = createRestartServerMutation();
	const stopMutation = createStopServerMutation();
	const startMutation = createStartServerMutation();
	const forceKillMutation = createForceKillServerMutation();

	let server = $derived(serverQuery.data);
	let availableActions = $derived(server ? getDetailAvailableActions(server.status) : []);

	let throughputHistory = $state<TimeSeriesPoint[]>([]);
	let latencyHistory = $state<LatencyPercentilePoint[]>([]);
	let lastMetricsUpdate = $state(0);

	const MAX_HISTORY_POINTS = 500;

	$effect(() => {
		if (metricsQuery.dataUpdatedAt > lastMetricsUpdate) {
			lastMetricsUpdate = metricsQuery.dataUpdatedAt;
			const metrics = metricsQuery.data;

			if (metrics && metrics.throughput !== undefined) {
				untrack(() => {
					const now = Date.now();
					let newThroughput = [
						...throughputHistory,
						{ timestamp: now, value: Number(metrics.throughput) }
					];
					if (newThroughput.length > MAX_HISTORY_POINTS)
						newThroughput = newThroughput.slice(-MAX_HISTORY_POINTS);
					throughputHistory = newThroughput;

					if (metrics.latency) {
						let newLatency = [
							...latencyHistory,
							{
								timestamp: now,
								p50: metrics.latency.p50 || 0,
								p90: metrics.latency.p90 || 0,
								p99: metrics.latency.p99 || 0
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

	// Mock slots for now since the API doesn't return them yet
	let slots = $derived<SlotState[]>(
		server
			? Array.from({ length: metricsQuery.data?.slots.total || 0 }).map((_, i) => ({
					id: `${i}`,
					status: i < (metricsQuery.data?.slots.processing || 0) ? 'processing' : 'idle'
				}))
			: []
	);

	let confirmAction = $state<DetailServerAction | null>(null);
	let pendingAction = $state<DetailServerAction | null>(null);

	function handleAction(action: DetailServerAction) {
		if (action === 'stop' || action === 'force_kill') {
			confirmAction = action;
		} else {
			executeAction(action);
		}
	}

	function executeAction(action: DetailServerAction) {
		pendingAction = action;
		const mutation =
			action === 'start'
				? startMutation
				: action === 'restart'
					? restartMutation
					: action === 'stop'
						? stopMutation
						: forceKillMutation;

		mutation.mutate(serverId, {
			onSettled: () => {
				pendingAction = null;
			}
		});
	}

	function confirmDialogText(action: DetailServerAction) {
		if (action === 'stop')
			return {
				title: 'Stop Server',
				message: `Are you sure you want to gracefully stop ${server?.name}?`
			};
		if (action === 'force_kill')
			return {
				title: 'Force Kill Server',
				message: `Are you sure you want to FORCE KILL ${server?.name}? This will instantly drop all active connections.`
			};
		return { title: '', message: '' };
	}
</script>

<svelte:head>
	<title>{server ? server.name : 'Server Detail'} | LlamaForge</title>
</svelte:head>

<div role="main" class="mx-auto max-w-[1400px] space-y-6">
	{#if serverQuery.isPending}
		<Skeleton width="300px" height="2rem" />
		<Skeleton height="300px" />
	{:else if serverQuery.isError || !server}
		<EmptyState
			title="Server not found"
			description="The server you are looking for does not exist or an error occurred."
			icon="❌"
		/>
	{:else}
		<div class="flex items-center justify-between border-b border-[var(--color-border)] pb-6">
			<div>
				<h1 tabindex="-1" class="text-3xl font-bold text-[var(--color-text-primary)]">
					{server.name}
				</h1>
				<div class="mt-2 flex items-center gap-4">
					<StatusBadge status={server.status} size="lg" />
					<span class="font-mono text-sm text-[var(--color-text-secondary)]">ID: {server.id}</span>
				</div>
			</div>
			<ServerActionBar {availableActions} {pendingAction} onAction={handleAction} />
		</div>

		<SlotGrid {slots} serverName={server.name} />

		<ServerMetrics metrics={metricsQuery.data} {throughputHistory} {latencyHistory} />

		<ConfirmDialog
			open={confirmAction !== null}
			onCancel={() => (confirmAction = null)}
			onConfirm={() => {
				if (confirmAction) executeAction(confirmAction);
				confirmAction = null;
			}}
			title={confirmAction ? confirmDialogText(confirmAction).title : ''}
			message={confirmAction ? confirmDialogText(confirmAction).message : ''}
			variant={confirmAction === 'force_kill' || confirmAction === 'stop' ? 'danger' : 'default'}
			confirmLabel={confirmAction === 'force_kill' ? 'Kill' : 'Stop'}
		/>
	{/if}
</div>
