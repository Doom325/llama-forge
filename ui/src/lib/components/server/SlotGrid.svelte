<script lang="ts">
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';

	export interface SlotState {
		id: string;
		status: 'idle' | 'processing' | 'loading';
		model?: string;
		requestId?: string;
	}

	let { slots, serverName }: { slots: SlotState[]; serverName: string } = $props();

	function getSlotBadgeStatus(status: SlotState['status']) {
		if (status === 'processing') return 'pending';
		if (status === 'loading') return 'reconnecting';
		return 'healthy'; // idle
	}

	function getSlotBadgeLabel(status: SlotState['status']) {
		if (status === 'processing') return 'Processing';
		if (status === 'loading') return 'Loading';
		return 'Idle';
	}

	function getSlotIcon(status: SlotState['status']) {
		if (status === 'processing') return '⚙️';
		if (status === 'loading') return '⏳';
		return '🟢'; // idle
	}
</script>

<div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6">
	<h3 class="mb-4 text-lg font-bold text-[var(--color-text-primary)]">
		Slots ({slots.filter((s) => s.status === 'processing').length} Active / {slots.length} Total)
	</h3>

	{#if slots.length === 0}
		<div class="text-sm text-[var(--color-text-secondary)]">
			No slots configured for {serverName}
		</div>
	{:else}
		<div class="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4">
			{#each slots as slot (slot.id)}
				<div
					class="flex flex-col gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] p-4"
				>
					<div class="flex items-center justify-between">
						<span class="text-sm font-medium text-[var(--color-text-secondary)]"
							>Slot {slot.id}</span
						>
						<StatusBadge
							status={getSlotBadgeStatus(slot.status)}
							customLabel={getSlotBadgeLabel(slot.status)}
							size="sm"
						/>
					</div>

					<div class="mt-2 flex items-center gap-2">
						<span class="text-xl" aria-hidden="true">{getSlotIcon(slot.status)}</span>
						<div class="flex flex-col text-xs text-[var(--color-text-primary)]">
							{#if slot.status === 'idle'}
								<span>Ready</span>
							{:else if slot.model}
								<span class="truncate font-mono">{slot.model}</span>
								{#if slot.requestId}
									<span class="truncate text-[var(--color-text-secondary)]">{slot.requestId}</span>
								{/if}
							{/if}
						</div>
					</div>
				</div>
			{/each}
		</div>
	{/if}
</div>
