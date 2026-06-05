<script lang="ts">
	// Presentational: renders wizard progress. Completed steps are navigable
	// buttons; the current step carries aria-current="step". Below 640px only
	// numbered circles show. Pure — no I/O, navigation is delegated upward.

	export interface IndicatorStep {
		label: string;
		status: 'complete' | 'current' | 'upcoming';
	}

	let {
		steps,
		onnavigate
	}: {
		steps: IndicatorStep[];
		onnavigate?: (index: number) => void;
	} = $props();

	let listEl = $state<HTMLOListElement | null>(null);

	function handleKeydown(event: KeyboardEvent) {
		if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
		const buttons = Array.from(
			listEl?.querySelectorAll<HTMLButtonElement>('button:not([disabled])') ?? []
		);
		const pos = buttons.indexOf(document.activeElement as HTMLButtonElement);
		if (pos === -1) return;
		event.preventDefault();
		const next = event.key === 'ArrowRight' ? pos + 1 : pos - 1;
		buttons[next]?.focus();
	}

	function circleClass(status: IndicatorStep['status']): string {
		if (status === 'complete') {
			return 'bg-[var(--color-status-healthy)] text-white border-[var(--color-status-healthy)]';
		}
		if (status === 'current') {
			return 'bg-[var(--color-focus-ring)] text-white border-[var(--color-focus-ring)]';
		}
		return 'bg-transparent text-[var(--color-text-secondary)] border-[var(--color-border)]';
	}
</script>

<nav aria-label="Setup progress">
	<ol bind:this={listEl} class="flex items-center gap-2 sm:gap-4">
		{#each steps as step, index (step.label)}
			<li class="flex items-center gap-2">
				{#if step.status === 'complete' && onnavigate}
					<button
						type="button"
						onclick={() => onnavigate(index)}
						onkeydown={handleKeydown}
						class="flex items-center gap-2 rounded focus:ring-2 focus:ring-[var(--color-focus-ring)] focus:outline-none"
					>
						<span
							class="flex h-7 w-7 items-center justify-center rounded-full border text-sm font-medium {circleClass(
								step.status
							)}"
							aria-hidden="true">{index + 1}</span
						>
						<span class="hidden text-sm font-medium text-[var(--color-text-primary)] sm:inline"
							>{step.label}</span
						>
						<span class="sr-only">{step.label} (completed)</span>
					</button>
				{:else}
					<span
						class="flex items-center gap-2"
						aria-current={step.status === 'current' ? 'step' : undefined}
					>
						<span
							class="flex h-7 w-7 items-center justify-center rounded-full border text-sm font-medium {circleClass(
								step.status
							)}"
							aria-hidden="true">{index + 1}</span
						>
						<span
							class="hidden text-sm font-medium sm:inline {step.status === 'current'
								? 'text-[var(--color-text-primary)]'
								: 'text-[var(--color-text-secondary)]'}">{step.label}</span
						>
						<span class="sr-only"
							>{step.label}{step.status === 'current' ? ' (current step)' : ''}</span
						>
					</span>
				{/if}
				{#if index < steps.length - 1}
					<span class="h-px w-4 bg-[var(--color-border)] sm:w-8" aria-hidden="true"></span>
				{/if}
			</li>
		{/each}
	</ol>
</nav>
