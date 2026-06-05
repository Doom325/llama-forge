<script lang="ts">
	// Presentational build lifecycle view. Renders from the BuildStatus
	// discriminant; emits intent via callback props. Pure — no I/O.
	import ProgressBar from '$lib/components/shared/ProgressBar.svelte';
	import LogViewer from '$lib/components/shared/LogViewer.svelte';
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import type { BuildPhase, BuildStatus } from '$lib/api/types';

	let {
		status,
		onretry,
		onskip,
		onchangebackend,
		oncancel
	}: {
		status: BuildStatus;
		onretry?: () => void;
		onskip?: () => void;
		onchangebackend?: () => void;
		oncancel?: () => void;
	} = $props();

	const phaseLabel: Record<BuildPhase, string> = {
		cloning: 'Cloning llama.cpp',
		configuring: 'Configuring',
		building: 'Compiling'
	};

	const secondaryButton =
		'rounded border border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';
</script>

{#if status.status === 'running'}
	<div class="space-y-3">
		<p class="text-sm font-medium text-[var(--color-text-primary)]">
			{phaseLabel[status.phase]}…
		</p>
		<ProgressBar value={status.percent} label={phaseLabel[status.phase]} />
		<LogViewer lines={status.logLines} label="Build log" />
		<button type="button" onclick={() => oncancel?.()} class={secondaryButton}>Cancel build</button>
	</div>
{:else if status.status === 'failed'}
	<div class="space-y-3">
		<ErrorBanner message={`Build failed during ${phaseLabel[status.phase]}: ${status.error}`} />
		<LogViewer lines={status.lastLines} label="Build error output" />
		<div class="flex flex-wrap gap-3">
			<button
				type="button"
				onclick={() => onretry?.()}
				class="rounded bg-[var(--color-accent-fill)] px-4 py-2 text-sm font-medium text-white focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:ring-offset-2 focus:outline-none"
				>Retry</button
			>
			<button type="button" onclick={() => onchangebackend?.()} class={secondaryButton}
				>Change backend</button
			>
			<button type="button" onclick={() => onskip?.()} class={secondaryButton}>Skip</button>
		</div>
	</div>
{:else if status.status === 'complete'}
	<div class="space-y-2">
		<div class="flex items-center gap-2">
			<StatusBadge status="healthy" size="sm" customLabel="Built" />
			<span class="text-sm font-medium text-[var(--color-text-primary)]">Build complete</span>
		</div>
		<p class="text-sm text-[var(--color-text-secondary)]">
			Installed to <code>{status.binary_path}</code> in {status.duration_secs}s.
		</p>
		{#if status.replaced_existing && status.backup_path}
			<p class="text-xs text-[var(--color-text-secondary)]">
				Previous binary backed up to <code>{status.backup_path}</code>.
			</p>
		{/if}
	</div>
{/if}
