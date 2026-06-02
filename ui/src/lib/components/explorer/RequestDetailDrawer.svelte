<script lang="ts">
	import type { RequestRecord } from '$lib/api/types';
	import { formatDuration } from '$lib/utils/format';
	import StatusBadge from '$lib/components/shared/StatusBadge.svelte';

	let {
		request,
		open,
		onClose
	}: {
		request: RequestRecord | null;
		open: boolean;
		onClose: () => void;
	} = $props();

	// Focus management: when drawer opens, focus close button. When it closes, we signal orchestrator.
	let closeButton: HTMLButtonElement | undefined = $state();

	$effect(() => {
		if (open && closeButton) {
			// Small delay to allow DOM transition if any
			setTimeout(() => closeButton?.focus(), 50);
		}
	});

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			onClose();
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
	<!-- Backdrop -->
	<div
		class="fixed inset-0 z-40 bg-black/50 transition-opacity"
		aria-hidden="true"
		onclick={onClose}
	></div>

	<!-- Drawer -->
	<div
		class="fixed inset-y-0 right-0 z-50 flex w-full max-w-md transform flex-col border-l border-[var(--color-border)] bg-[var(--color-bg-primary)] shadow-xl transition-transform duration-300 ease-in-out sm:max-w-lg md:max-w-xl"
		role="dialog"
		aria-modal="true"
		aria-labelledby="drawer-title"
	>
		<div class="flex items-center justify-between border-b border-[var(--color-border)] p-4">
			<h2 id="drawer-title" class="text-lg font-bold text-[var(--color-text-primary)]">
				Request Details
			</h2>
			<button
				bind:this={closeButton}
				type="button"
				class="rounded-md p-2 text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-secondary)] hover:text-[var(--color-text-primary)] focus:outline-none focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]"
				onclick={onClose}
				aria-label="Close details"
			>
				<svg class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="2"
						d="M6 18L18 6M6 6l12 12"
					/>
				</svg>
			</button>
		</div>

		<div class="flex-1 space-y-6 overflow-y-auto p-4">
			{#if request}
				<div class="space-y-4">
					<div class="grid grid-cols-2 gap-4">
						<div class="space-y-1">
							<span
								class="text-xs font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
								>ID</span
							>
							<div class="font-mono text-sm">{request.id}</div>
						</div>
						<div class="space-y-1">
							<span
								class="text-xs font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
								>Status</span
							>
							<div>
								<StatusBadge
									status={request.status === 'success'
										? 'healthy'
										: request.status === 'error'
											? 'failed'
											: 'pending'}
									customLabel={request.status}
								/>
							</div>
						</div>
						<div class="space-y-1">
							<span
								class="text-xs font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
								>Model</span
							>
							<div class="text-sm">{request.model}</div>
						</div>
						<div class="space-y-1">
							<span
								class="text-xs font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
								>Task Type</span
							>
							<div class="text-sm capitalize">{request.taskType}</div>
						</div>
						<div class="space-y-1">
							<span
								class="text-xs font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
								>Timestamp</span
							>
							<div class="text-sm">{new Date(request.timestamp).toLocaleString()}</div>
						</div>
						<div class="space-y-1">
							<span
								class="text-xs font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
								>Duration</span
							>
							<div class="text-sm">{formatDuration(request.duration)}</div>
						</div>
					</div>

					<div class="border-t border-[var(--color-border)] pt-4">
						<h3
							class="mb-2 text-sm font-semibold tracking-wider text-[var(--color-text-secondary)] uppercase"
						>
							Payload (Mock)
						</h3>
						<pre
							class="overflow-x-auto rounded-md border border-[var(--color-border)] bg-[var(--color-bg-secondary)] p-4 font-mono text-xs whitespace-pre-wrap">
{JSON.stringify(request, null, 2)}
						</pre>
					</div>
				</div>
			{:else}
				<div class="mt-10 text-center text-[var(--color-text-secondary)]">No request selected</div>
			{/if}
		</div>
	</div>
{/if}
