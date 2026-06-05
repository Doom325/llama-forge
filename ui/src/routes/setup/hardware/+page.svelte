<script lang="ts">
	// HardwareStep orchestrator (08_SETUP_WIZARD.md §2.1/4): fires hardware
	// detection, shows cards on success, and offers a manual-entry fallback on
	// error or when detection is slow.
	import { createHardwareQuery } from '$lib/api/queries';
	import { queryClient } from '$lib/api/queryClient';
	import { useSetupStore } from '$lib/stores/setup.svelte';
	import HardwareCard from '$lib/components/setup/HardwareCard.svelte';
	import HardwareManualEntry from '$lib/components/setup/HardwareManualEntry.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import type { HardwareInfo } from '$lib/api/types';

	const store = useSetupStore();
	const hw = createHardwareQuery(store.sessionId);

	let manualMode = $state(false);
	let showManualPrompt = $state(false);

	// After 10s of pending detection, surface the manual-entry escape hatch.
	$effect(() => {
		if (!hw.isPending) return;
		const timer = setTimeout(() => (showManualPrompt = true), 10_000);
		return () => clearTimeout(timer);
	});

	function applyManual(hwInfo: HardwareInfo) {
		queryClient.setQueryData(['setup', 'hardware'], hwInfo);
		manualMode = false;
	}

	const secondaryButton =
		'rounded border border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';
</script>

<h1 tabindex="-1" class="text-2xl font-semibold text-[var(--color-text-primary)]">Hardware</h1>
<p class="mt-2 text-[var(--color-text-secondary)]">
	Detecting your CPU, memory, and GPUs to recommend a backend.
</p>

<div class="mt-6">
	{#if manualMode}
		<HardwareManualEntry onsubmit={applyManual} />
	{:else if hw.isPending}
		<div class="grid gap-4 sm:grid-cols-2">
			<Skeleton height="5rem" />
			<Skeleton height="5rem" />
			<Skeleton height="5rem" />
		</div>
		{#if showManualPrompt}
			<button type="button" onclick={() => (manualMode = true)} class="{secondaryButton} mt-4">
				Detection is taking a while — enter hardware manually
			</button>
		{/if}
	{:else if hw.isError}
		<ErrorBanner
			message="Hardware detection failed."
			action={{ label: 'Retry', onClick: () => hw.refetch() }}
		/>
		<button type="button" onclick={() => (manualMode = true)} class="{secondaryButton} mt-4">
			Enter hardware manually
		</button>
	{:else if hw.data}
		<div class="grid gap-4 sm:grid-cols-2">
			{#each hw.data.gpus as gpu (gpu.index)}
				<HardwareCard
					type="gpu"
					label={gpu.name}
					details={`${gpu.vram_mb} MB VRAM`}
					status="detected"
				/>
			{/each}
			<HardwareCard
				type="cpu"
				label={hw.data.cpu.name}
				details={`${hw.data.cpu.cores_physical} cores / ${hw.data.cpu.cores_logical} threads`}
				status="detected"
			/>
			<HardwareCard
				type="ram"
				label="System RAM"
				details={`${hw.data.ram_mb} MB`}
				status="detected"
			/>
		</div>
		<p class="mt-4 text-sm text-[var(--color-text-secondary)]">
			Recommended backend: <strong>{hw.data.recommended_backend.toUpperCase()}</strong>
		</p>
	{/if}
</div>
