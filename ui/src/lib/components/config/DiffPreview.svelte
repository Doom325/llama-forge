<script lang="ts">
	import { Dialog } from 'bits-ui';

	let {
		original,
		modified,
		format = 'json',
		open,
		onClose
	}: {
		original: unknown;
		modified: unknown;
		format?: 'json' | 'toml';
		open: boolean;
		onClose: () => void;
	} = $props();

	interface DiffEntry {
		path: string;
		kind: 'added' | 'removed' | 'changed';
		before: string | null;
		after: string | null;
	}

	function flatten(value: unknown, prefix: string, out: Map<string, unknown>) {
		if (value !== null && typeof value === 'object') {
			const obj = value as Record<string, unknown>;
			const keys = Object.keys(obj);
			if (keys.length === 0) {
				out.set(prefix, value);
				return;
			}
			for (const key of keys) {
				const next = prefix ? `${prefix}.${key}` : key;
				flatten(obj[key], next, out);
			}
			return;
		}
		out.set(prefix, value);
	}

	function format_leaf(value: unknown): string {
		return JSON.stringify(value);
	}

	let diff = $derived.by(() => {
		const before = new Map<string, unknown>();
		const after = new Map<string, unknown>();
		flatten(original, '', before);
		flatten(modified, '', after);

		const paths = new Set<string>([...before.keys(), ...after.keys()]);
		const entries: DiffEntry[] = [];
		for (const path of [...paths].sort()) {
			const hasBefore = before.has(path);
			const hasAfter = after.has(path);
			const beforeVal = before.get(path);
			const afterVal = after.get(path);

			if (hasBefore && hasAfter) {
				if (format_leaf(beforeVal) !== format_leaf(afterVal)) {
					entries.push({
						path,
						kind: 'changed',
						before: format_leaf(beforeVal),
						after: format_leaf(afterVal)
					});
				}
			} else if (hasAfter) {
				entries.push({ path, kind: 'added', before: null, after: format_leaf(afterVal) });
			} else {
				entries.push({ path, kind: 'removed', before: format_leaf(beforeVal), after: null });
			}
		}
		return entries;
	});
</script>

<Dialog.Root
	{open}
	onOpenChange={(v) => {
		if (!v) onClose();
	}}
>
	<Dialog.Portal>
		<Dialog.Overlay class="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm" />
		<Dialog.Content
			class="fixed top-1/2 left-1/2 z-50 max-h-[80vh] w-full max-w-2xl -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6 shadow-xl"
		>
			<Dialog.Title class="text-xl font-bold text-[var(--color-text-primary)]">
				Review Changes
			</Dialog.Title>
			<Dialog.Description class="mt-1 text-sm text-[var(--color-text-secondary)]">
				Pending changes ({format}) to be saved.
			</Dialog.Description>

			<div class="mt-4 space-y-2">
				{#if diff.length === 0}
					<p class="text-sm text-[var(--color-text-secondary)]">No changes.</p>
				{:else}
					{#each diff as entry (entry.path)}
						<div class="rounded-md border border-[var(--color-border)] p-3 text-sm">
							<div class="font-mono font-semibold text-[var(--color-text-primary)]">
								{entry.path}
							</div>
							<div class="mt-1 flex flex-col gap-1 font-mono text-xs sm:flex-row sm:items-center">
								{#if entry.before !== null}
									<span class="text-[var(--color-status-failed)]">- {entry.before}</span>
								{/if}
								{#if entry.after !== null}
									<span class="text-[var(--color-status-healthy)]">+ {entry.after}</span>
								{/if}
							</div>
						</div>
					{/each}
				{/if}
			</div>

			<div class="mt-6 flex justify-end">
				<Dialog.Close
					class="rounded-md border border-[var(--color-border)] px-4 py-2 font-medium hover:bg-[var(--color-bg-secondary)]"
				>
					Close
				</Dialog.Close>
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
