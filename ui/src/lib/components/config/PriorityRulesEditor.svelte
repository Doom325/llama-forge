<script lang="ts">
	import type { PriorityRulesConfig } from '$lib/api/types';
	import { createSaveConfigMutation } from '$lib/api/queries';
	import { createConfigForm, type ValidationErrors } from '$lib/utils/configForm.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import DiffPreview from './DiffPreview.svelte';

	let {
		initial,
		etag,
		reload
	}: {
		initial: PriorityRulesConfig;
		etag: string | null;
		reload: () => Promise<{ data: PriorityRulesConfig; etag: string | null }>;
	} = $props();

	function validate(data: PriorityRulesConfig): ValidationErrors {
		const errors: ValidationErrors = {};
		data.task_types.forEach((rule, index) => {
			if (rule.priority < 0 || rule.priority > 1000) {
				errors[`task_types.${index}.priority`] = 'Priority must be between 0 and 1000.';
			}
		});
		data.fair_share.forEach((weight, index) => {
			if (weight.weight < 0) {
				errors[`fair_share.${index}.weight`] = 'Weight cannot be negative.';
			}
		});
		if (data.starvation.boost_start_after_secs < 0) {
			errors['starvation.boost_start_after_secs'] = 'Boost start cannot be negative.';
		}
		if (data.starvation.max_boost < 0) {
			errors['starvation.max_boost'] = 'Max boost cannot be negative.';
		}
		return errors;
	}

	const saveMutation = createSaveConfigMutation('priority');
	// `initial`/`etag` are consumed once at construction; reading them here is
	// intentional and not a missed reactive dependency.
	// svelte-ignore state_referenced_locally
	const form = createConfigForm<PriorityRulesConfig>({
		initial,
		etag,
		validate,
		save: saveMutation.mutateAsync
	});

	let diffOpen = $state(false);
	let reloadButton = $state<HTMLButtonElement | null>(null);

	$effect(() => {
		if (form.conflict) {
			reloadButton?.focus();
		}
	});

	async function handleReload() {
		const fresh = await reload();
		form.rebase(fresh.data, fresh.etag);
	}

	function addTaskType() {
		form.current.task_types = [...form.current.task_types, { task_type: '', priority: 0 }];
	}
	function removeTaskType(index: number) {
		form.current.task_types = form.current.task_types.filter((_, i) => i !== index);
	}
	function addFairShare() {
		form.current.fair_share = [...form.current.fair_share, { requestor: '', weight: 1 }];
	}
	function removeFairShare(index: number) {
		form.current.fair_share = form.current.fair_share.filter((_, i) => i !== index);
	}

	const inputClass =
		'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none';
	const labelClass = 'block text-sm font-medium text-[var(--color-text-secondary)]';
	const cardClass =
		'space-y-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6';
	const removeBtnClass =
		'rounded-md border border-[var(--color-border)] px-3 py-1.5 text-sm font-medium text-[var(--color-status-failed)] hover:bg-[var(--color-bg-secondary)]';
</script>

{#if form.conflict}
	<div class="mb-4">
		<ErrorBanner
			message="Configuration was updated by another source."
			variant="warning"
			action={{ label: 'Reload', onClick: handleReload }}
		/>
		<button bind:this={reloadButton} type="button" class="sr-only" onclick={handleReload}>
			Reload configuration
		</button>
	</div>
{/if}

<form
	class="space-y-6"
	onsubmit={(e) => {
		e.preventDefault();
		form.submit();
	}}
>
	<div class={cardClass}>
		<h2 class="text-sm font-semibold text-[var(--color-text-primary)]">Task type priorities</h2>
		{#each form.current.task_types as rule, index (index)}
			<div class="flex items-start gap-3">
				<div class="flex-1 space-y-1">
					<label for="task-{index}-type" class={labelClass}>Task type</label>
					<input
						id="task-{index}-type"
						type="text"
						class={inputClass}
						bind:value={rule.task_type}
					/>
				</div>
				<div class="w-32 space-y-1">
					<label for="task-{index}-priority" class={labelClass}>Priority</label>
					<input
						id="task-{index}-priority"
						type="number"
						class={inputClass}
						bind:value={rule.priority}
						aria-invalid={!!form.errors[`task_types.${index}.priority`]}
						aria-describedby={form.errors[`task_types.${index}.priority`]
							? `task-${index}-priority-error`
							: undefined}
					/>
					{#if form.errors[`task_types.${index}.priority`]}
						<p
							id="task-{index}-priority-error"
							role="alert"
							class="text-xs text-[var(--color-status-failed)]"
						>
							{form.errors[`task_types.${index}.priority`]}
						</p>
					{/if}
				</div>
				<button type="button" class="{removeBtnClass} mt-6" onclick={() => removeTaskType(index)}
					>Remove</button
				>
			</div>
		{/each}
		<button
			type="button"
			class="rounded-md border border-dashed border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)]"
			onclick={addTaskType}
		>
			+ Add task type
		</button>
	</div>

	<div class={cardClass}>
		<h2 class="text-sm font-semibold text-[var(--color-text-primary)]">Fair share weights</h2>
		{#each form.current.fair_share as weight, index (index)}
			<div class="flex items-start gap-3">
				<div class="flex-1 space-y-1">
					<label for="fair-{index}-requestor" class={labelClass}>Requestor</label>
					<input
						id="fair-{index}-requestor"
						type="text"
						class={inputClass}
						bind:value={weight.requestor}
					/>
				</div>
				<div class="w-32 space-y-1">
					<label for="fair-{index}-weight" class={labelClass}>Weight</label>
					<input
						id="fair-{index}-weight"
						type="number"
						class={inputClass}
						bind:value={weight.weight}
						aria-invalid={!!form.errors[`fair_share.${index}.weight`]}
						aria-describedby={form.errors[`fair_share.${index}.weight`]
							? `fair-${index}-weight-error`
							: undefined}
					/>
					{#if form.errors[`fair_share.${index}.weight`]}
						<p
							id="fair-{index}-weight-error"
							role="alert"
							class="text-xs text-[var(--color-status-failed)]"
						>
							{form.errors[`fair_share.${index}.weight`]}
						</p>
					{/if}
				</div>
				<button type="button" class="{removeBtnClass} mt-6" onclick={() => removeFairShare(index)}
					>Remove</button
				>
			</div>
		{/each}
		<button
			type="button"
			class="rounded-md border border-dashed border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)]"
			onclick={addFairShare}
		>
			+ Add weight
		</button>
	</div>

	<div class={cardClass}>
		<h2 class="text-sm font-semibold text-[var(--color-text-primary)]">Starvation</h2>
		<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
			<div class="space-y-1">
				<label for="boost_start_after_secs" class={labelClass}>Boost start after (secs)</label>
				<input
					id="boost_start_after_secs"
					type="number"
					class={inputClass}
					bind:value={form.current.starvation.boost_start_after_secs}
					aria-invalid={!!form.errors['starvation.boost_start_after_secs']}
					aria-describedby={form.errors['starvation.boost_start_after_secs']
						? 'boost_start_after_secs-error'
						: undefined}
				/>
				{#if form.errors['starvation.boost_start_after_secs']}
					<p
						id="boost_start_after_secs-error"
						role="alert"
						class="text-xs text-[var(--color-status-failed)]"
					>
						{form.errors['starvation.boost_start_after_secs']}
					</p>
				{/if}
			</div>
			<div class="space-y-1">
				<label for="max_boost" class={labelClass}>Max boost</label>
				<input
					id="max_boost"
					type="number"
					class={inputClass}
					bind:value={form.current.starvation.max_boost}
					aria-invalid={!!form.errors['starvation.max_boost']}
					aria-describedby={form.errors['starvation.max_boost'] ? 'max_boost-error' : undefined}
				/>
				{#if form.errors['starvation.max_boost']}
					<p id="max_boost-error" role="alert" class="text-xs text-[var(--color-status-failed)]">
						{form.errors['starvation.max_boost']}
					</p>
				{/if}
			</div>
		</div>
	</div>

	<div class="flex items-center justify-end gap-3 border-t border-[var(--color-border)] pt-4">
		{#if form.isDirty}
			<button
				type="button"
				class="rounded-md border border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)]"
				onclick={() => (diffOpen = true)}
			>
				Review Changes
			</button>
			<button
				type="button"
				class="rounded-md border border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)]"
				onclick={() => form.reset()}
			>
				Reset
			</button>
		{/if}
		<button
			type="submit"
			class="rounded-md bg-[var(--color-focus-ring)] px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
			disabled={form.isSubmitting}
		>
			{form.isSubmitting ? 'Saving…' : 'Save'}
		</button>
	</div>
</form>

<DiffPreview
	original={form.saved}
	modified={form.current}
	open={diffOpen}
	onClose={() => (diffOpen = false)}
/>
