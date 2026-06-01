<script lang="ts">
	export type Status =
		| 'healthy'
		| 'degraded'
		| 'failed'
		| 'pending'
		| 'reconnecting'
		| 'disconnected';

	let { status, size = 'md' }: { status: Status; size?: 'sm' | 'md' | 'lg' } = $props();

	const statusConfig = {
		healthy: {
			icon: '✓',
			label: 'Healthy',
			colorClass:
				'text-[var(--color-status-healthy)] bg-[var(--color-status-healthy)]/10 border-[var(--color-status-healthy)]/20'
		},
		degraded: {
			icon: '⚠',
			label: 'Degraded',
			colorClass:
				'text-[var(--color-status-degraded)] bg-[var(--color-status-degraded)]/10 border-[var(--color-status-degraded)]/20'
		},
		failed: {
			icon: '✕',
			label: 'Failed',
			colorClass:
				'text-[var(--color-status-failed)] bg-[var(--color-status-failed)]/10 border-[var(--color-status-failed)]/20'
		},
		disconnected: {
			icon: '✕',
			label: 'Disconnected',
			colorClass:
				'text-[var(--color-status-failed)] bg-[var(--color-status-failed)]/10 border-[var(--color-status-failed)]/20'
		},
		pending: {
			icon: '⟳',
			label: 'Pending',
			colorClass:
				'text-[var(--color-status-pending)] bg-[var(--color-status-pending)]/10 border-[var(--color-status-pending)]/20'
		},
		reconnecting: {
			icon: '⟳',
			label: 'Reconnecting',
			colorClass:
				'text-[var(--color-status-degraded)] bg-[var(--color-status-degraded)]/10 border-[var(--color-status-degraded)]/20'
		}
	};

	let config = $derived(statusConfig[status]);

	let sizeClasses = $derived(
		size === 'sm'
			? 'text-xs px-2 py-0.5'
			: size === 'lg'
				? 'text-base px-3 py-1'
				: 'text-sm px-2.5 py-0.5'
	);
</script>

<span
	class="inline-flex items-center gap-1.5 rounded-full border font-medium {config.colorClass} {sizeClasses}"
	aria-label="Status: {config.label}"
>
	<span aria-hidden="true" class="font-bold select-none">{config.icon}</span>
	<span>{config.label}</span>
</span>
