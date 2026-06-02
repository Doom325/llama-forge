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

	let focusedIndex = $state(0);

	function handleKeydown(e: KeyboardEvent, index: number) {
		if (e.key === 'ArrowDown') {
			e.preventDefault();
			focusedIndex = (index + 1) % availableActions.length;
			const el = document.getElementById(`action-btn-${availableActions[focusedIndex]}`);
			el?.focus();
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			focusedIndex = (index - 1 + availableActions.length) % availableActions.length;
			const el = document.getElementById(`action-btn-${availableActions[focusedIndex]}`);
			el?.focus();
		} else if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			if (pendingAction === null) onAction(availableActions[index]!);
		}
	}

	function handleFocus(index: number) {
		focusedIndex = index;
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
				{@const disabled = pendingAction !== null}
				<button
					id="action-btn-{action}"
					type="button"
					role="menuitem"
					{disabled}
					class="w-full rounded-md px-3 py-2 text-left text-sm font-medium transition-colors focus:outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)] {getActionClass(
						action
					)}"
					class:opacity-50={disabled}
					class:cursor-not-allowed={disabled}
					onclick={() => !disabled && onAction(action)}
					onkeydown={(e) => handleKeydown(e, index)}
					onfocus={() => handleFocus(index)}
					aria-disabled={disabled}
					tabindex={index === focusedIndex ? 0 : -1}
				>
					{getActionLabel(action, isPending)}
				</button>
			{/each}
		</div>
	{/if}
</div>
