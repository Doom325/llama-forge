<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		children,
		itemCount
	}: {
		children: Snippet<[number]>;
		itemCount: number;
	} = $props();

	let gridContainer: HTMLDivElement | null = null;
	let focusedIndex = $state(0);

	function getColumns(): number {
		if (!gridContainer || typeof window === 'undefined') return 1;
		const style = window.getComputedStyle(gridContainer);
		const columnsStr = style.gridTemplateColumns;
		if (!columnsStr || columnsStr === 'none') return 1;
		return columnsStr.split(' ').length;
	}

	function handleKeydown(e: KeyboardEvent) {
		if (!gridContainer || itemCount === 0) return;

		const cardElements = Array.from(
			gridContainer.querySelectorAll('[role="gridcell"]')
		) as HTMLElement[];
		if (cardElements.length === 0) return;

		let newIndex = focusedIndex;
		let handled = false;
		const columns = getColumns();

		switch (e.key) {
			case 'ArrowRight':
				newIndex = Math.min(focusedIndex + 1, itemCount - 1);
				handled = true;
				break;
			case 'ArrowLeft':
				newIndex = Math.max(focusedIndex - 1, 0);
				handled = true;
				break;
			case 'ArrowDown':
				newIndex = Math.min(focusedIndex + columns, itemCount - 1);
				handled = true;
				break;
			case 'ArrowUp':
				newIndex = Math.max(focusedIndex - columns, 0);
				handled = true;
				break;
			case 'Home':
				if (e.ctrlKey || e.metaKey) {
					newIndex = 0; // Top left
				} else {
					// First in current row
					newIndex = Math.floor(focusedIndex / columns) * columns;
				}
				handled = true;
				break;
			case 'End':
				if (e.ctrlKey || e.metaKey) {
					newIndex = itemCount - 1; // Bottom right
				} else {
					// Last in current row
					newIndex = Math.min(
						Math.floor(focusedIndex / columns) * columns + columns - 1,
						itemCount - 1
					);
				}
				handled = true;
				break;
		}

		if (handled) {
			e.preventDefault();
			const targetEl = cardElements[newIndex];
			if (newIndex !== focusedIndex && targetEl) {
				focusedIndex = newIndex;
				targetEl.focus();
			}
		}
	}

	// Reset focus index if items are removed
	$effect(() => {
		if (focusedIndex >= itemCount && itemCount > 0) {
			focusedIndex = itemCount - 1;

			// If focus was inside the grid and is now lost due to removal, refocus the new index
			if (typeof document !== 'undefined' && document.activeElement === document.body) {
				// Schedule focus for next tick to ensure Svelte has updated the DOM tabindex
				setTimeout(() => {
					const cells = gridContainer?.querySelectorAll('[role="gridcell"]');
					const targetEl = cells?.[focusedIndex] as HTMLElement | undefined;
					if (targetEl) targetEl.focus();
				}, 0);
			}
		} else if (itemCount === 0) {
			focusedIndex = 0;
		}
	});
</script>

<div
	bind:this={gridContainer}
	role="grid"
	tabindex="-1"
	aria-label="Servers Grid"
	class="grid w-full gap-6"
	style="grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));"
	onkeydown={handleKeydown}
>
	<div role="row" class="contents">
		{@render children(focusedIndex)}
	</div>
</div>
