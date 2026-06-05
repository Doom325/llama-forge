<script lang="ts">
	// Presentational: one detected hardware component (GPU/CPU/RAM) with its
	// detection status shown via StatusBadge. Pure — no I/O.
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';
	import type { ServerStatus } from '$lib/api/types';

	type DetectionStatus = 'detected' | 'not_detected' | 'error';

	let {
		type,
		label,
		details,
		status,
		icon
	}: {
		type: 'gpu' | 'cpu' | 'ram';
		label: string;
		details: string;
		status: DetectionStatus;
		icon?: string;
	} = $props();

	const badgeStatus: Record<DetectionStatus, ServerStatus> = {
		detected: 'healthy',
		not_detected: 'degraded',
		error: 'failed'
	};
	const defaultIcon: Record<'gpu' | 'cpu' | 'ram', string> = { gpu: '🎮', cpu: '🧠', ram: '💾' };
</script>

<div class="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-4">
	<div class="flex items-start justify-between gap-3">
		<div class="flex items-center gap-2">
			<span aria-hidden="true" class="text-lg">{icon ?? defaultIcon[type]}</span>
			<div>
				<h3 class="text-sm font-medium text-[var(--color-text-primary)]">{label}</h3>
				<p class="text-xs text-[var(--color-text-secondary)]">{details}</p>
			</div>
		</div>
		<StatusBadge status={badgeStatus[status]} size="sm" />
	</div>
</div>
