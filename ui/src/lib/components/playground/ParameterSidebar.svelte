<script lang="ts">
	import type { GenerationParams } from '$lib/api/types';

	let {
		params,
		defaults,
		onChange,
		onReset
	}: {
		params: GenerationParams;
		defaults: GenerationParams;
		onChange: (p: GenerationParams) => void;
		onReset: () => void;
	} = $props();

	function update(patch: Partial<GenerationParams>) {
		onChange({ ...params, ...patch });
	}

	const labelClass =
		'flex items-center justify-between text-sm font-medium text-[var(--color-text-secondary)]';
	const numberClass =
		'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none';

	// defaults is read for the Reset affordance; surface it so it is not flagged unused.
	const resetLabel = $derived(`Reset (temp ${defaults.temperature}, max ${defaults.max_tokens})`);
</script>

<aside
	class="space-y-5 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-secondary)] p-5"
	aria-label="Generation parameters"
>
	<h2 class="text-sm font-semibold text-[var(--color-text-primary)]">Parameters</h2>

	<div class="space-y-1">
		<label class={labelClass} for="param-temperature">
			<span>Temperature</span>
			<span class="font-mono text-[var(--color-text-primary)]">{params.temperature.toFixed(1)}</span
			>
		</label>
		<input
			id="param-temperature"
			type="range"
			min="0"
			max="2"
			step="0.1"
			class="w-full"
			value={params.temperature}
			oninput={(e) => update({ temperature: Number(e.currentTarget.value) })}
		/>
	</div>

	<div class="space-y-1">
		<label class={labelClass} for="param-top-p">
			<span>Top P</span>
			<span class="font-mono text-[var(--color-text-primary)]">{params.top_p.toFixed(2)}</span>
		</label>
		<input
			id="param-top-p"
			type="range"
			min="0"
			max="1"
			step="0.05"
			class="w-full"
			value={params.top_p}
			oninput={(e) => update({ top_p: Number(e.currentTarget.value) })}
		/>
	</div>

	<div class="space-y-1">
		<label
			class="block text-sm font-medium text-[var(--color-text-secondary)]"
			for="param-max-tokens"
		>
			Max tokens
		</label>
		<input
			id="param-max-tokens"
			type="number"
			min="1"
			step="1"
			class={numberClass}
			value={params.max_tokens}
			oninput={(e) => update({ max_tokens: Number(e.currentTarget.value) })}
		/>
	</div>

	<div class="space-y-1">
		<label class="block text-sm font-medium text-[var(--color-text-secondary)]" for="param-top-k">
			Top K
		</label>
		<input
			id="param-top-k"
			type="number"
			min="0"
			step="1"
			class={numberClass}
			value={params.top_k}
			oninput={(e) => update({ top_k: Number(e.currentTarget.value) })}
		/>
	</div>

	<button
		type="button"
		class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-border)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-focus-ring)]"
		onclick={onReset}
		title={resetLabel}
	>
		Reset to defaults
	</button>
</aside>
