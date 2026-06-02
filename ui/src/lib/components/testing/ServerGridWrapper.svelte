<script lang="ts">
	import ServerGrid from '../dashboard/ServerGrid.svelte';

	let { itemCount = 4 }: { itemCount?: number } = $props();
</script>

<ServerGrid {itemCount}>
	{#snippet children(focusedIndex: number)}
		{#each Array.from({ length: itemCount }).keys() as i (i)}
			<div role="gridcell" tabindex={i === focusedIndex ? 0 : -1}>
				Cell {i}
				<button type="button" tabindex={i === focusedIndex ? 0 : -1}>Action {i}</button>
			</div>
		{/each}
	{/snippet}
</ServerGrid>
