<script lang="ts">
	import type { DetailServerAction } from '$lib/api/types';

	let {
		availableActions,
		pendingAction,
		onAction
	}: {
		availableActions: DetailServerAction[];
		pendingAction: DetailServerAction | null;
		onAction: (action: DetailServerAction) => void;
	} = $props();

	function getActionLabel(action: DetailServerAction, isPending: boolean): string {
		if (isPending) {
			if (action === 'start') return 'Starting...';
			if (action === 'stop') return 'Stopping...';
			if (action === 'restart') return 'Restarting...';
			if (action === 'force_kill') return 'Killing...';
		}
		if (action === 'start') return 'Start Server';
		if (action === 'stop') return 'Stop Server';
		if (action === 'restart') return 'Restart Server';
		if (action === 'force_kill') return 'Force Kill';
		return action;
	}

	function getActionClass(action: DetailServerAction): string {
		if (action === 'force_kill' || action === 'stop') {
			return 'text-[var(--color-status-failed)] hover:bg-[var(--color-status-failed)]/10 focus:bg-[var(--color-status-failed)]/10';
		}
		if (action === 'start') {
			return 'text-[var(--color-status-healthy)] hover:bg-[var(--color-status-healthy)]/10 focus:bg-[var(--color-status-healthy)]/10';
		}
		return 'text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)] focus:bg-[var(--color-bg-secondary)]';
	}

	function handleKeydown(e: KeyboardEvent, index: number) {
		if (e.key === 'ArrowDown') {
			e.preventDefault();
			const next = (index + 1) % availableActions.length;
			const el = document.getElementById(`action-btn-${availableActions[next]}`);
			el?.focus();
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			const prev = (index - 1 + availableActions.length) % availableActions.length;
			const el = document.getElementById(`action-btn-${availableActions[prev]}`);
			el?.focus();
		}
	}
</script>

<div
	class="relative inline-block w-48 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-2 text-left"
>
	<div
		class="mb-1 px-3 py-2 text-xs font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
	>
		Actions
	</div>
	{#if availableActions.length === 0}
		<div class="px-3 py-2 text-sm text-[var(--color-text-secondary)] italic">
			No actions available
		</div>
	{:else}
		<div role="menu" aria-label="Server actions" class="flex flex-col gap-1">
			{#each availableActions as action, index (action)}
				{@const isPending = pendingAction === action}
				<button
					id="action-btn-{action}"
					type="button"
					role="menuitem"
					disabled={pendingAction !== null}
					class="w-full rounded-md px-3 py-2 text-left text-sm font-medium transition-colors focus:outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)] disabled:cursor-not-allowed disabled:opacity-50 {getActionClass(
						action
					)}"
					onclick={() => onAction(action)}
					onkeydown={(e) => handleKeydown(e, index)}
					aria-disabled={pendingAction !== null}
					tabindex={index === 0 ? 0 : -1}
				>
					{getActionLabel(action, isPending)}
				</button>
			{/each}
		</div>
	{/if}
</div>
