<script lang="ts">
	let {
		title,
		description,
		icon,
		action
	}: {
		title: string;
		description: string;
		icon?: string;
		action?: { label: string; onClick: () => void };
	} = $props();
</script>

<div
	class="flex flex-col items-center justify-center p-8 text-center text-[var(--color-text-secondary)]"
>
	{#if icon}
		<div class="mb-4 text-4xl" aria-hidden="true">{icon}</div>
	{/if}
	<h3 class="mb-2 text-lg font-semibold text-[var(--color-text-primary)]">{title}</h3>
	<p class="mb-6 max-w-md">{description}</p>
	{#if action}
		<button
			type="button"
			class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg-secondary)] px-4 py-2 font-medium text-[var(--color-text-primary)] hover:bg-[var(--color-border)]"
			onclick={action.onClick}
		>
			{action.label}
		</button>
	{/if}
</div>
