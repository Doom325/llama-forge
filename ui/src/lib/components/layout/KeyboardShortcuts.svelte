<script lang="ts">
	import { Dialog } from 'bits-ui';
	import { onMount } from 'svelte';

	let open = $state(false);

	onMount(() => {
		const handleKeydown = (e: KeyboardEvent) => {
			const target = e.target as HTMLElement;
			const isInputNode =
				target.tagName === 'INPUT' ||
				target.tagName === 'TEXTAREA' ||
				target.tagName === 'SELECT' ||
				target.isContentEditable;

			if (e.key === '?' && !e.ctrlKey && !e.metaKey && !e.altKey && !isInputNode) {
				open = true;
			}
		};
		window.addEventListener('keydown', handleKeydown);
		return () => window.removeEventListener('keydown', handleKeydown);
	});
</script>

<Dialog.Root bind:open>
	<Dialog.Trigger
		class="inline-flex h-9 w-9 items-center justify-center rounded-md border border-[var(--color-border)] hover:bg-[var(--color-bg-secondary)]"
		aria-label="Keyboard shortcuts"
	>
		<span aria-hidden="true">?</span>
	</Dialog.Trigger>
	<Dialog.Portal>
		<Dialog.Overlay class="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm" />
		<Dialog.Content
			class="fixed top-1/2 left-1/2 z-50 w-full max-w-md -translate-x-1/2 -translate-y-1/2 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6 shadow-xl"
		>
			<Dialog.Title class="text-xl font-bold">Keyboard Shortcuts</Dialog.Title>
			<Dialog.Description class="sr-only">List of available keyboard shortcuts</Dialog.Description>
			<div class="mt-4 space-y-4">
				<div class="flex items-center justify-between">
					<span>Toggle Sidebar</span>
					<kbd class="rounded border border-[var(--color-border)] px-2 py-1 font-mono text-sm"
						>Ctrl + B</kbd
					>
				</div>
				<div class="flex items-center justify-between">
					<span>Show Shortcuts</span>
					<kbd class="rounded border border-[var(--color-border)] px-2 py-1 font-mono text-sm"
						>?</kbd
					>
				</div>
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
