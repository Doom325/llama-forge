import { render } from '@testing-library/svelte';
import { describe, it, expect, vi } from 'vitest';
import ServerGridWrapper from '../testing/ServerGridWrapper.svelte';
import userEvent from '@testing-library/user-event';
import { tick } from 'svelte';

describe('ServerGrid', () => {
	it('supports roving tabindex and arrow key navigation', async () => {
		const user = userEvent.setup();
		const { container } = render(ServerGridWrapper, { itemCount: 4 });

		// Wait for effect to initialize focus index if any
		const gridContainer = container.querySelector('[role="grid"]') as HTMLElement;
		vi.spyOn(window, 'getComputedStyle').mockImplementation((elem: Element) => {
			if (elem === gridContainer) {
				return { gridTemplateColumns: '320px 320px' } as CSSStyleDeclaration;
			}
			return {} as CSSStyleDeclaration;
		});

		const cells = Array.from(container.querySelectorAll('[role="gridcell"]')) as HTMLElement[];
		expect(cells.length).toBeGreaterThan(0);

		const firstCell = cells[0];
		expect(firstCell).toBeDefined();
		if (!firstCell) return;

		// First cell should be focusable
		expect(firstCell).toHaveAttribute('tabindex', '0');

		firstCell.focus();
		expect(document.activeElement).toBe(firstCell);

		// Right arrow
		await user.keyboard('{ArrowRight}');
		expect(document.activeElement).toBe(cells[1]);

		// Down arrow (moves down 1 row, +2 items)
		await user.keyboard('{ArrowDown}');
		expect(document.activeElement).toBe(cells[3]);

		// Left arrow
		await user.keyboard('{ArrowLeft}');
		expect(document.activeElement).toBe(cells[2]);

		// Up arrow
		await user.keyboard('{ArrowUp}');
		expect(document.activeElement).toBe(cells[0]);

		// End
		await user.keyboard('{End}');
		expect(document.activeElement).toBe(cells[1]); // End of row

		// Home
		await user.keyboard('{Home}');
		expect(document.activeElement).toBe(cells[0]); // Start of row
	});

	it('makes action buttons reachable only for the focused card', async () => {
		const user = userEvent.setup();
		const { container } = render(ServerGridWrapper, { itemCount: 2 });

		const cells = Array.from(container.querySelectorAll('[role="gridcell"]')) as HTMLElement[];
		const buttons = Array.from(container.querySelectorAll('button')) as HTMLButtonElement[];

		expect(cells[0]).toHaveAttribute('tabindex', '0');
		expect(buttons[0]).toHaveAttribute('tabindex', '0');

		expect(cells[1]).toHaveAttribute('tabindex', '-1');
		expect(buttons[1]).toHaveAttribute('tabindex', '-1');

		// Focus first cell
		expect(cells[0]).toBeDefined();
		cells[0]?.focus();
		expect(document.activeElement).toBe(cells[0]);

		// Tab to button inside the cell
		await user.tab();
		expect(document.activeElement).toBe(buttons[0]);
	});

	it('refocuses correct index when items are removed', async () => {
		const user = userEvent.setup();
		const { container, rerender } = render(ServerGridWrapper, { itemCount: 4 });

		let cells = Array.from(container.querySelectorAll('[role="gridcell"]')) as HTMLElement[];
		cells[0]?.focus();
		await user.keyboard('{ArrowRight}');
		await user.keyboard('{ArrowRight}');
		await user.keyboard('{ArrowRight}');

		expect(document.activeElement).toBe(cells[3]);

		rerender({ itemCount: 2 });
		await tick();
		await new Promise((r) => setTimeout(r, 50)); // wait for effects and focus

		// Svelte effect will update focusedIndex and refocus natively
		cells = Array.from(container.querySelectorAll('[role="gridcell"]')) as HTMLElement[];
		expect(document.activeElement).toBe(cells[1]);
	});
});
