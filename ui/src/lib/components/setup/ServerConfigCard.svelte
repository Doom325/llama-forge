<script lang="ts">
	// Presentational editor for one server's config draft. Controlled: the
	// current `values` come in as a prop (recommendation merged with overrides by
	// the orchestrator) and every edit is emitted via `onchange`. Pure — no state.
	import type { ConfigRecommendation, DiscoveredModel, ServerConfigDraft } from '$lib/api/types';

	let {
		model,
		recommendation,
		values,
		onchange
	}: {
		model: DiscoveredModel;
		recommendation: ConfigRecommendation;
		values: ServerConfigDraft;
		onchange: (next: ServerConfigDraft) => void;
	} = $props();

	function update(patch: Partial<ServerConfigDraft>) {
		onchange({ ...values, ...patch });
	}

	const portValid = $derived(values.port > 0 && values.port < 65536);
	const ctxValid = $derived(values.ctx_size > 0);

	const fieldClass =
		'mt-1 w-full rounded border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';
	const labelClass = 'text-xs font-medium text-[var(--color-text-primary)]';
</script>

<div
	class="space-y-4 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-4"
>
	<div>
		<h3 class="font-mono text-sm font-medium text-[var(--color-text-primary)]">{model.filename}</h3>
		<p class="text-xs text-[var(--color-text-secondary)]">
			~{recommendation.estimated_vram_mb} MB VRAM estimated
		</p>
	</div>

	<div class="grid gap-4 sm:grid-cols-2">
		<div>
			<label class={labelClass} for="alias-{model.id}">Alias</label>
			<input
				id="alias-{model.id}"
				class={fieldClass}
				value={values.alias}
				oninput={(e) => update({ alias: e.currentTarget.value })}
			/>
		</div>
		<div>
			<label class={labelClass} for="port-{model.id}">Port</label>
			<input
				id="port-{model.id}"
				type="number"
				class={fieldClass}
				value={values.port}
				aria-invalid={!portValid}
				oninput={(e) => update({ port: Number(e.currentTarget.value) })}
			/>
			{#if !portValid}
				<p class="mt-1 text-xs text-[var(--color-status-failed)]">Port must be 1–65535.</p>
			{/if}
		</div>
		<div>
			<label class={labelClass} for="gpu-{model.id}">GPU layers</label>
			<input
				id="gpu-{model.id}"
				type="number"
				class={fieldClass}
				value={values.gpu_layers}
				oninput={(e) => update({ gpu_layers: Number(e.currentTarget.value) })}
			/>
			{#if recommendation.gpu_layers !== undefined}
				<p class="mt-1 text-xs text-[var(--color-text-secondary)]">
					Recommended: {recommendation.gpu_layers}
				</p>
			{/if}
		</div>
		<div>
			<label class={labelClass} for="ctx-{model.id}">Context size</label>
			<input
				id="ctx-{model.id}"
				type="number"
				class={fieldClass}
				value={values.ctx_size}
				aria-invalid={!ctxValid}
				oninput={(e) => update({ ctx_size: Number(e.currentTarget.value) })}
			/>
			{#if !ctxValid}
				<p class="mt-1 text-xs text-[var(--color-status-failed)]">Context size must be positive.</p>
			{/if}
		</div>
		<div>
			<label class={labelClass} for="slots-{model.id}">Parallel slots</label>
			<input
				id="slots-{model.id}"
				type="number"
				class={fieldClass}
				value={values.parallel_slots}
				oninput={(e) => update({ parallel_slots: Number(e.currentTarget.value) })}
			/>
		</div>
		<div>
			<label class={labelClass} for="flash-{model.id}">Flash attention</label>
			<input
				id="flash-{model.id}"
				class={fieldClass}
				value={values.flash_attn}
				oninput={(e) => update({ flash_attn: e.currentTarget.value })}
			/>
		</div>
		<div>
			<label class={labelClass} for="ck-{model.id}">KV cache (K)</label>
			<input
				id="ck-{model.id}"
				class={fieldClass}
				value={values.cache_type_k}
				oninput={(e) => update({ cache_type_k: e.currentTarget.value })}
			/>
		</div>
		<div>
			<label class={labelClass} for="cv-{model.id}">KV cache (V)</label>
			<input
				id="cv-{model.id}"
				class={fieldClass}
				value={values.cache_type_v}
				oninput={(e) => update({ cache_type_v: e.currentTarget.value })}
			/>
		</div>
	</div>
</div>
