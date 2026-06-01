<script lang="ts">
	import { Dialog } from 'bits-ui';

	let {
		open = $bindable(false),
		title,
		message,
		confirmLabel = 'Confirm',
		cancelLabel = 'Cancel',
		variant = 'default',
		onConfirm,
		onCancel
	}: {
		open?: boolean;
		title: string;
		message: string;
		confirmLabel?: string;
		cancelLabel?: string;
		variant?: 'danger' | 'default';
		onConfirm: () => void;
		onCancel?: () => void;
	} = $props();

	function handleConfirm() {
		onConfirm();
		open = false;
	}

	let confirmButtonClass = $derived(
		variant === 'danger'
			? 'bg-[var(--color-status-failed)] text-white hover:opacity-90'
			: 'bg-[var(--color-focus-ring)] text-white hover:opacity-90'
	);
</script>

<Dialog.Root
	bind:open
	onOpenChange={(v) => {
		if (!v && onCancel) onCancel();
	}}
>
	<Dialog.Portal>
		<Dialog.Overlay class="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm" />
		<Dialog.Content
			class="fixed top-1/2 left-1/2 z-50 w-full max-w-md -translate-x-1/2 -translate-y-1/2 rounded-xl border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-6 shadow-xl"
		>
			<Dialog.Title class="text-xl font-bold">{title}</Dialog.Title>
			<Dialog.Description class="mt-3 text-[var(--color-text-secondary)]">
				{message}
			</Dialog.Description>

			<div class="mt-6 flex justify-end gap-3">
				<Dialog.Close
					class="rounded-md border border-[var(--color-border)] px-4 py-2 font-medium hover:bg-[var(--color-bg-secondary)]"
				>
					{cancelLabel}
				</Dialog.Close>
				<button
					type="button"
					class="rounded-md px-4 py-2 font-medium {confirmButtonClass}"
					onclick={handleConfirm}
				>
					{confirmLabel}
				</button>
			</div>
		</Dialog.Content>
	</Dialog.Portal>
</Dialog.Root>
