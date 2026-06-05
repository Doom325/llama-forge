<script lang="ts">
	// Presentational manual-entry fallback used when hardware detection fails or
	// is too slow. Validates inline and emits a HardwareInfo via `onsubmit`.
	import type { HardwareInfo } from '$lib/api/types';

	let { onsubmit }: { onsubmit: (hw: HardwareInfo) => void } = $props();

	let gpuName = $state('');
	let vramMb = $state<number | null>(null);
	let cpuCores = $state<number | null>(null);
	let ramMb = $state<number | null>(null);
	let touched = $state(false);

	const vramValid = $derived(vramMb !== null && vramMb >= 0);
	const coresValid = $derived(cpuCores !== null && cpuCores > 0);
	const ramValid = $derived(ramMb !== null && ramMb > 0);
	const valid = $derived(vramValid && coresValid && ramValid);

	function submit(event: SubmitEvent) {
		event.preventDefault();
		touched = true;
		if (!valid || vramMb === null || cpuCores === null || ramMb === null) return;
		const hw: HardwareInfo = {
			gpus: vramMb > 0 ? [{ index: 0, name: gpuName.trim() || 'GPU', vram_mb: vramMb }] : [],
			cpu: { name: 'CPU (manual)', cores_physical: cpuCores, cores_logical: cpuCores },
			ram_mb: ramMb,
			recommended_backend: vramMb > 0 ? 'cuda' : 'cpu'
		};
		onsubmit(hw);
	}

	const fieldClass =
		'mt-1 w-full rounded border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';
</script>

<form onsubmit={submit} class="space-y-4" novalidate>
	<div>
		<label for="hw-gpu" class="text-sm font-medium text-[var(--color-text-primary)]">
			GPU name <span class="text-[var(--color-text-secondary)]">(optional)</span>
		</label>
		<input id="hw-gpu" bind:value={gpuName} class={fieldClass} placeholder="e.g. RTX 4090" />
	</div>

	<div>
		<label for="hw-vram" class="text-sm font-medium text-[var(--color-text-primary)]">
			GPU VRAM (MB) <span class="text-[var(--color-text-secondary)]">(0 if no GPU)</span>
		</label>
		<input
			id="hw-vram"
			type="number"
			min="0"
			bind:value={vramMb}
			class={fieldClass}
			aria-invalid={touched && !vramValid}
		/>
		{#if touched && !vramValid}
			<p class="mt-1 text-xs text-[var(--color-status-failed)]">Enter VRAM in MB (0 or more).</p>
		{/if}
	</div>

	<div>
		<label for="hw-cores" class="text-sm font-medium text-[var(--color-text-primary)]"
			>CPU cores</label
		>
		<input
			id="hw-cores"
			type="number"
			min="1"
			bind:value={cpuCores}
			class={fieldClass}
			aria-invalid={touched && !coresValid}
		/>
		{#if touched && !coresValid}
			<p class="mt-1 text-xs text-[var(--color-status-failed)]">Enter a positive core count.</p>
		{/if}
	</div>

	<div>
		<label for="hw-ram" class="text-sm font-medium text-[var(--color-text-primary)]"
			>System RAM (MB)</label
		>
		<input
			id="hw-ram"
			type="number"
			min="1"
			bind:value={ramMb}
			class={fieldClass}
			aria-invalid={touched && !ramValid}
		/>
		{#if touched && !ramValid}
			<p class="mt-1 text-xs text-[var(--color-status-failed)]">Enter RAM in MB (positive).</p>
		{/if}
	</div>

	<button
		type="submit"
		class="rounded bg-[var(--color-accent-fill)] px-5 py-2 text-sm font-medium text-white focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:ring-offset-2 focus:outline-none"
	>
		Use this hardware
	</button>
</form>
