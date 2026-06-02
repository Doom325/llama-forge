<script lang="ts">
	import type { ServersConfig, ServerConfigItem } from '$lib/api/types';
	import { createSaveConfigMutation } from '$lib/api/queries';
	import { createConfigForm, type ValidationErrors } from '$lib/utils/configForm.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import DiffPreview from './DiffPreview.svelte';

	let {
		initial,
		etag,
		reload
	}: {
		initial: ServersConfig;
		etag: string | null;
		reload: () => Promise<{ data: ServersConfig; etag: string | null }>;
	} = $props();

	function validate(data: ServersConfig): ValidationErrors {
		const errors: ValidationErrors = {};
		data.servers.forEach((server, index) => {
			if (!server.id || server.id.trim() === '') {
				errors[`servers.${index}.id`] = 'Server id is required.';
			}
			if (!Number.isInteger(server.port) || server.port < 1 || server.port > 65535) {
				errors[`servers.${index}.port`] = 'Port must be between 1 and 65535.';
			}
		});
		return errors;
	}

	const saveMutation = createSaveConfigMutation('servers');
	// `initial`/`etag` are consumed once at construction; reading them here is
	// intentional and not a missed reactive dependency.
	// svelte-ignore state_referenced_locally
	const form = createConfigForm<ServersConfig>({
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

	function addServer() {
		const blank: ServerConfigItem = {
			id: '',
			alias: '',
			model_path: '',
			port: 8080,
			gpu_layers: 0,
			parallel_slots: 1,
			ctx_size: 4096
		};
		form.current.servers = [...form.current.servers, blank];
	}

	function removeServer(index: number) {
		form.current.servers = form.current.servers.filter((_, i) => i !== index);
	}

	const inputClass =
		'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none';
	const labelClass = 'block text-sm font-medium text-[var(--color-text-secondary)]';
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
	<div class="space-y-4">
		{#each form.current.servers as server, index (index)}
			<div
				class="space-y-4 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6"
			>
				<div class="flex items-center justify-between">
					<h2 class="text-sm font-semibold text-[var(--color-text-primary)]">
						Server {index + 1}
					</h2>
					<button
						type="button"
						class="rounded-md border border-[var(--color-border)] px-3 py-1.5 text-sm font-medium text-[var(--color-status-failed)] hover:bg-[var(--color-bg-secondary)]"
						onclick={() => removeServer(index)}
					>
						Remove
					</button>
				</div>

				<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
					<div class="space-y-1">
						<label for="server-{index}-id" class={labelClass}>ID</label>
						<input
							id="server-{index}-id"
							type="text"
							class={inputClass}
							bind:value={server.id}
							aria-invalid={!!form.errors[`servers.${index}.id`]}
							aria-describedby={form.errors[`servers.${index}.id`]
								? `server-${index}-id-error`
								: undefined}
						/>
						{#if form.errors[`servers.${index}.id`]}
							<p
								id="server-{index}-id-error"
								role="alert"
								class="text-xs text-[var(--color-status-failed)]"
							>
								{form.errors[`servers.${index}.id`]}
							</p>
						{/if}
					</div>

					<div class="space-y-1">
						<label for="server-{index}-alias" class={labelClass}>Alias</label>
						<input
							id="server-{index}-alias"
							type="text"
							class={inputClass}
							bind:value={server.alias}
						/>
					</div>

					<div class="space-y-1 sm:col-span-2">
						<label for="server-{index}-model_path" class={labelClass}>Model path</label>
						<input
							id="server-{index}-model_path"
							type="text"
							class={inputClass}
							bind:value={server.model_path}
						/>
					</div>

					<div class="space-y-1">
						<label for="server-{index}-port" class={labelClass}>Port</label>
						<input
							id="server-{index}-port"
							type="number"
							class={inputClass}
							bind:value={server.port}
							aria-invalid={!!form.errors[`servers.${index}.port`]}
							aria-describedby={form.errors[`servers.${index}.port`]
								? `server-${index}-port-error`
								: undefined}
						/>
						{#if form.errors[`servers.${index}.port`]}
							<p
								id="server-{index}-port-error"
								role="alert"
								class="text-xs text-[var(--color-status-failed)]"
							>
								{form.errors[`servers.${index}.port`]}
							</p>
						{/if}
					</div>

					<div class="space-y-1">
						<label for="server-{index}-gpu_layers" class={labelClass}>GPU layers</label>
						<input
							id="server-{index}-gpu_layers"
							type="number"
							class={inputClass}
							bind:value={server.gpu_layers}
						/>
					</div>

					<div class="space-y-1">
						<label for="server-{index}-parallel_slots" class={labelClass}>Parallel slots</label>
						<input
							id="server-{index}-parallel_slots"
							type="number"
							class={inputClass}
							bind:value={server.parallel_slots}
						/>
					</div>

					<div class="space-y-1">
						<label for="server-{index}-ctx_size" class={labelClass}>Context size</label>
						<input
							id="server-{index}-ctx_size"
							type="number"
							class={inputClass}
							bind:value={server.ctx_size}
						/>
					</div>
				</div>
			</div>
		{/each}

		<button
			type="button"
			class="rounded-md border border-dashed border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-bg-secondary)]"
			onclick={addServer}
		>
			+ Add server
		</button>
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
