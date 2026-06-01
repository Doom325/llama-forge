<script lang="ts">
	let {
		message,
		variant = 'info',
		dismissible = false,
		action
	}: {
		message: string;
		variant?: 'error' | 'warning' | 'info';
		dismissible?: boolean;
		action?: { label: string; onClick: () => void };
	} = $props();

	let visible = $state(true);

	const config = {
		error: {
			icon: '✕',
			colorClass:
				'bg-[var(--color-status-failed)]/10 text-[var(--color-status-failed)] border-[var(--color-status-failed)]/20',
			role: 'alert'
		},
		warning: {
			icon: '⚠',
			colorClass:
				'bg-[var(--color-status-degraded)]/10 text-[var(--color-status-degraded)] border-[var(--color-status-degraded)]/20',
			role: 'status'
		},
		info: {
			icon: 'ℹ',
			colorClass:
				'bg-[var(--color-status-pending)]/10 text-[var(--color-status-pending)] border-[var(--color-status-pending)]/20',
			role: 'status'
		}
	};

	let currentConfig = $derived(config[variant]);

	$effect(() => {
		if (message) {
			visible = true;
		}
	});

	function handleDismiss() {
		visible = false;
	}
</script>

{#if visible}
	<div
		role={currentConfig.role}
		class="flex items-center gap-3 rounded-lg border p-4 {currentConfig.colorClass}"
	>
		<span aria-hidden="true" class="text-xl font-bold">{currentConfig.icon}</span>
		<div class="flex-1 font-medium">{message}</div>
		{#if action}
			<button
				type="button"
				class="rounded-md border border-current px-3 py-1.5 text-sm font-semibold hover:bg-current/10"
				onclick={action.onClick}
			>
				{action.label}
			</button>
		{/if}
		{#if dismissible}
			<button
				type="button"
				aria-label="Dismiss"
				class="ml-2 rounded-md p-1 hover:bg-current/10"
				onclick={handleDismiss}
			>
				<span aria-hidden="true">✕</span>
			</button>
		{/if}
	</div>
{/if}
