<script lang="ts">
	// Presentational build setup form. Emits the build request via `onstart`
	// after a confirmation dialog (building clones + compiles llama.cpp).
	import { untrack } from 'svelte';
	import ConfirmDialog from '$lib/components/shared/ConfirmDialog.svelte';
	import type { SetupBackend } from '$lib/api/types';

	let {
		detectedBackend,
		onstart
	}: {
		detectedBackend: SetupBackend;
		onstart: (req: { backend: SetupBackend; install_dir: string; jobs: number }) => void;
	} = $props();

	// Seed the editable select from the detected backend's initial value; the
	// user may then change it independently.
	let backend = $state<SetupBackend>(untrack(() => detectedBackend));
	let installDir = $state('');
	let jobs = $state<number>(0);
	let confirmOpen = $state(false);

	const backends: SetupBackend[] = ['cuda', 'metal', 'vulkan', 'cpu'];
	const canBuild = $derived(installDir.trim().length > 0);

	const fieldClass =
		'mt-1 w-full rounded border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';

	function requestBuild() {
		if (canBuild) confirmOpen = true;
	}
	function confirmBuild() {
		confirmOpen = false;
		onstart({ backend, install_dir: installDir.trim(), jobs: Math.max(0, jobs) });
	}
</script>

<div class="space-y-4">
	<div>
		<label for="build-backend" class="text-sm font-medium text-[var(--color-text-primary)]"
			>Backend</label
		>
		<select id="build-backend" bind:value={backend} class={fieldClass}>
			{#each backends as option (option)}
				<option value={option}>{option.toUpperCase()}</option>
			{/each}
		</select>
	</div>

	<div>
		<label for="build-dir" class="text-sm font-medium text-[var(--color-text-primary)]"
			>Install directory</label
		>
		<input id="build-dir" bind:value={installDir} class={fieldClass} placeholder="/usr/local/bin" />
	</div>

	<div>
		<label for="build-jobs" class="text-sm font-medium text-[var(--color-text-primary)]">
			Parallel jobs <span class="text-[var(--color-text-secondary)]">(0 = auto)</span>
		</label>
		<input id="build-jobs" type="number" min="0" bind:value={jobs} class={fieldClass} />
	</div>

	<button
		type="button"
		onclick={requestBuild}
		disabled={!canBuild}
		class="rounded bg-[var(--color-accent-fill)] px-5 py-2 text-sm font-medium text-white focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:ring-offset-2 focus:outline-none disabled:cursor-not-allowed disabled:opacity-40"
	>
		Build llama-server
	</button>
</div>

<ConfirmDialog
	open={confirmOpen}
	title="Build llama-server?"
	message="This clones llama.cpp and compiles it locally. It may take several minutes and use significant CPU and disk."
	confirmLabel="Start build"
	onConfirm={confirmBuild}
	onCancel={() => (confirmOpen = false)}
/>
