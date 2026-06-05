<script lang="ts">
	// Presentational: where to write the config + whether to overwrite. Pure —
	// value in, changes emitted via callbacks.
	let {
		path,
		force,
		placeholder = '~/.config/llamaforge/llamaforge.toml',
		onpathchange,
		onforcechange
	}: {
		path: string;
		force: boolean;
		placeholder?: string;
		onpathchange: (value: string) => void;
		onforcechange: (value: boolean) => void;
	} = $props();
</script>

<div class="space-y-3">
	<div>
		<label for="save-path" class="text-sm font-medium text-[var(--color-text-primary)]">
			Save configuration to
		</label>
		<input
			id="save-path"
			value={path}
			{placeholder}
			oninput={(e) => onpathchange(e.currentTarget.value)}
			class="mt-1 w-full rounded border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 font-mono text-sm text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none"
		/>
	</div>
	<label class="flex items-center gap-2 text-sm text-[var(--color-text-secondary)]">
		<input
			type="checkbox"
			checked={force}
			onchange={(e) => onforcechange(e.currentTarget.checked)}
		/>
		Overwrite if the file already exists
	</label>
</div>
