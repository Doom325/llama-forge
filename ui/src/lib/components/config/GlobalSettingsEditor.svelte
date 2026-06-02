<script lang="ts">
	import type { GlobalSettings, LogLevel, LogFormat } from '$lib/api/types';
	import { createSaveConfigMutation } from '$lib/api/queries';
	import { createConfigForm, type ValidationErrors } from '$lib/utils/configForm.svelte';
	import ErrorBanner from '$lib/components/shared/ErrorBanner.svelte';
	import DiffPreview from './DiffPreview.svelte';

	let {
		initial,
		etag,
		reload
	}: {
		initial: GlobalSettings;
		etag: string | null;
		reload: () => Promise<{ data: GlobalSettings; etag: string | null }>;
	} = $props();

	function validate(data: GlobalSettings): ValidationErrors {
		const errors: ValidationErrors = {};
		if (!data.bind_address || data.bind_address.trim() === '') {
			errors.bind_address = 'Bind address is required.';
		}
		if (!Number.isInteger(data.grpc_port) || data.grpc_port < 1 || data.grpc_port > 65535) {
			errors.grpc_port = 'gRPC port must be between 1 and 65535.';
		}
		if (!Number.isInteger(data.http_port) || data.http_port < 1 || data.http_port > 65535) {
			errors.http_port = 'HTTP port must be between 1 and 65535.';
		}
		if (data.metrics_retention_days < 1) {
			errors.metrics_retention_days = 'Retention must be at least 1 day.';
		}
		if (data.queue_max_depth < 1) {
			errors.queue_max_depth = 'Queue max depth must be at least 1.';
		}
		if (data.queue_default_timeout_secs < 1) {
			errors.queue_default_timeout_secs = 'Timeout must be at least 1 second.';
		}
		if (data.restart_stagger_secs < 0) {
			errors.restart_stagger_secs = 'Restart stagger cannot be negative.';
		}
		return errors;
	}

	const saveMutation = createSaveConfigMutation('global');
	// `initial`/`etag` are consumed once at construction; reading them here is
	// intentional and not a missed reactive dependency.
	// svelte-ignore state_referenced_locally
	const form = createConfigForm<GlobalSettings>({
		initial,
		etag,
		validate,
		save: saveMutation.mutateAsync
	});

	let diffOpen = $state(false);
	let reloadButton = $state<HTMLButtonElement | null>(null);

	// Track which fields have been blurred so errors only show after interaction.
	let touched = $state<Record<string, boolean>>({});
	function markTouched(field: string) {
		touched = { ...touched, [field]: true };
	}

	const logLevels: LogLevel[] = ['trace', 'debug', 'info', 'warn', 'error'];
	const logFormats: LogFormat[] = ['pretty', 'json', 'compact'];

	// Move focus to the Reload button when a conflict is detected.
	$effect(() => {
		if (form.conflict) {
			reloadButton?.focus();
		}
	});

	async function handleReload() {
		const fresh = await reload();
		form.rebase(fresh.data, fresh.etag);
		touched = {};
	}

	const inputClass =
		'w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-[var(--color-focus-ring)] focus:ring-1 focus:ring-[var(--color-focus-ring)] focus:outline-none';
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
	class="space-y-6 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6"
	onsubmit={(e) => {
		e.preventDefault();
		form.submit();
	}}
>
	<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
		<div class="space-y-1">
			<label for="bind_address" class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>Bind address</label
			>
			<input
				id="bind_address"
				type="text"
				class={inputClass}
				bind:value={form.current.bind_address}
				onblur={() => markTouched('bind_address')}
				aria-invalid={touched.bind_address && !!form.errors.bind_address}
				aria-describedby={form.errors.bind_address ? 'bind_address-error' : undefined}
			/>
			{#if touched.bind_address && form.errors.bind_address}
				<p id="bind_address-error" role="alert" class="text-xs text-[var(--color-status-failed)]">
					{form.errors.bind_address}
				</p>
			{/if}
		</div>

		<div class="space-y-1">
			<label for="grpc_port" class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>gRPC port</label
			>
			<input
				id="grpc_port"
				type="number"
				class={inputClass}
				bind:value={form.current.grpc_port}
				onblur={() => markTouched('grpc_port')}
				aria-invalid={touched.grpc_port && !!form.errors.grpc_port}
				aria-describedby={form.errors.grpc_port ? 'grpc_port-error' : undefined}
			/>
			{#if touched.grpc_port && form.errors.grpc_port}
				<p id="grpc_port-error" role="alert" class="text-xs text-[var(--color-status-failed)]">
					{form.errors.grpc_port}
				</p>
			{/if}
		</div>

		<div class="space-y-1">
			<label for="http_port" class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>HTTP port</label
			>
			<input
				id="http_port"
				type="number"
				class={inputClass}
				bind:value={form.current.http_port}
				onblur={() => markTouched('http_port')}
				aria-invalid={touched.http_port && !!form.errors.http_port}
				aria-describedby={form.errors.http_port ? 'http_port-error' : undefined}
			/>
			{#if touched.http_port && form.errors.http_port}
				<p id="http_port-error" role="alert" class="text-xs text-[var(--color-status-failed)]">
					{form.errors.http_port}
				</p>
			{/if}
		</div>

		<div class="space-y-1">
			<label for="log_level" class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>Log level</label
			>
			<select id="log_level" class={inputClass} bind:value={form.current.log_level}>
				{#each logLevels as level (level)}
					<option value={level}>{level}</option>
				{/each}
			</select>
		</div>

		<div class="space-y-1">
			<label for="log_format" class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>Log format</label
			>
			<select id="log_format" class={inputClass} bind:value={form.current.log_format}>
				{#each logFormats as fmt (fmt)}
					<option value={fmt}>{fmt}</option>
				{/each}
			</select>
		</div>

		<div class="space-y-1">
			<label
				for="metrics_retention_days"
				class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>Metrics retention (days)</label
			>
			<input
				id="metrics_retention_days"
				type="number"
				class={inputClass}
				bind:value={form.current.metrics_retention_days}
				onblur={() => markTouched('metrics_retention_days')}
				aria-invalid={touched.metrics_retention_days && !!form.errors.metrics_retention_days}
				aria-describedby={form.errors.metrics_retention_days
					? 'metrics_retention_days-error'
					: undefined}
			/>
			{#if touched.metrics_retention_days && form.errors.metrics_retention_days}
				<p
					id="metrics_retention_days-error"
					role="alert"
					class="text-xs text-[var(--color-status-failed)]"
				>
					{form.errors.metrics_retention_days}
				</p>
			{/if}
		</div>

		<div class="space-y-1">
			<label
				for="queue_max_depth"
				class="block text-sm font-medium text-[var(--color-text-secondary)]">Queue max depth</label
			>
			<input
				id="queue_max_depth"
				type="number"
				class={inputClass}
				bind:value={form.current.queue_max_depth}
				onblur={() => markTouched('queue_max_depth')}
				aria-invalid={touched.queue_max_depth && !!form.errors.queue_max_depth}
				aria-describedby={form.errors.queue_max_depth ? 'queue_max_depth-error' : undefined}
			/>
			{#if touched.queue_max_depth && form.errors.queue_max_depth}
				<p
					id="queue_max_depth-error"
					role="alert"
					class="text-xs text-[var(--color-status-failed)]"
				>
					{form.errors.queue_max_depth}
				</p>
			{/if}
		</div>

		<div class="space-y-1">
			<label
				for="queue_default_timeout_secs"
				class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>Queue default timeout (secs)</label
			>
			<input
				id="queue_default_timeout_secs"
				type="number"
				class={inputClass}
				bind:value={form.current.queue_default_timeout_secs}
				onblur={() => markTouched('queue_default_timeout_secs')}
				aria-invalid={touched.queue_default_timeout_secs &&
					!!form.errors.queue_default_timeout_secs}
				aria-describedby={form.errors.queue_default_timeout_secs
					? 'queue_default_timeout_secs-error'
					: undefined}
			/>
			{#if touched.queue_default_timeout_secs && form.errors.queue_default_timeout_secs}
				<p
					id="queue_default_timeout_secs-error"
					role="alert"
					class="text-xs text-[var(--color-status-failed)]"
				>
					{form.errors.queue_default_timeout_secs}
				</p>
			{/if}
		</div>

		<div class="space-y-1">
			<label
				for="restart_stagger_secs"
				class="block text-sm font-medium text-[var(--color-text-secondary)]"
				>Restart stagger (secs)</label
			>
			<input
				id="restart_stagger_secs"
				type="number"
				class={inputClass}
				bind:value={form.current.restart_stagger_secs}
				onblur={() => markTouched('restart_stagger_secs')}
				aria-invalid={touched.restart_stagger_secs && !!form.errors.restart_stagger_secs}
				aria-describedby={form.errors.restart_stagger_secs
					? 'restart_stagger_secs-error'
					: undefined}
			/>
			{#if touched.restart_stagger_secs && form.errors.restart_stagger_secs}
				<p
					id="restart_stagger_secs-error"
					role="alert"
					class="text-xs text-[var(--color-status-failed)]"
				>
					{form.errors.restart_stagger_secs}
				</p>
			{/if}
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
