<script lang="ts">
	import { toastStore } from '$lib/stores/toast.svelte';
	import type { ToastType } from '$lib/stores/toast.svelte';

	const typeStyles: Record<ToastType, string> = {
		success:
			'bg-[var(--color-status-healthy)]/10 text-[var(--color-status-healthy)] border-[var(--color-status-healthy)]/20',
		error:
			'bg-[var(--color-status-failed)]/10 text-[var(--color-status-failed)] border-[var(--color-status-failed)]/20',
		warning:
			'bg-[var(--color-status-degraded)]/10 text-[var(--color-status-degraded)] border-[var(--color-status-degraded)]/20',
		info: 'bg-[var(--color-status-pending)]/10 text-[var(--color-status-pending)] border-[var(--color-status-pending)]/20'
	};
</script>

<div
	role="status"
	aria-live="polite"
	class="pointer-events-none fixed right-4 bottom-4 z-50 flex flex-col gap-2"
>
	{#each toastStore.toasts as toast (toast.id)}
		<div class="pointer-events-auto rounded-md border px-4 py-3 shadow-lg {typeStyles[toast.type]}">
			{toast.message}
		</div>
	{/each}
</div>
