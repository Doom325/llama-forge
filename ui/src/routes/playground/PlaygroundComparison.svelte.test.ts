import { render, screen } from '@testing-library/svelte';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import userEvent from '@testing-library/user-event';
import * as queries from '$lib/api/queries';
import type { ModelInfo } from '$lib/api/types';

vi.mock('$app/state', () => ({
	page: {
		url: new URL('http://localhost/playground?temperature=0.7&max_tokens=256&top_p=0.9&top_k=40')
	}
}));

vi.mock('$app/navigation', () => ({
	goto: vi.fn()
}));

// The highlighter dynamically imports shiki; stub it so mount stays synchronous.
vi.mock('$lib/utils/markdown', () => ({
	loadHighlighter: vi.fn(() => Promise.resolve()),
	renderMarkdown: (s: string) => s
}));

const models: ModelInfo[] = [
	{ id: 'a', name: 'Model A', loaded: true, assignedServers: [], capabilities: [] },
	{ id: 'b', name: 'Model B', loaded: true, assignedServers: [], capabilities: [] }
];

type ModelsQuery = ReturnType<typeof queries.createModelsQuery>;

function stubMatchMedia(matches: boolean) {
	Object.defineProperty(window, 'matchMedia', {
		writable: true,
		configurable: true,
		value: (query: string) => ({
			matches,
			media: query,
			onchange: null,
			addEventListener: vi.fn(),
			removeEventListener: vi.fn(),
			addListener: vi.fn(),
			removeListener: vi.fn(),
			dispatchEvent: vi.fn()
		})
	});
}

describe('PlaygroundPage comparison mode', () => {
	beforeEach(() => {
		vi.spyOn(queries, 'createModelsQuery').mockReturnValue({
			isPending: false,
			isError: false,
			data: models
		} as unknown as ModelsQuery);
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('renders a proper ARIA tabs widget in the narrow layout when comparing', async () => {
		stubMatchMedia(true); // < 1280px
		const user = userEvent.setup();
		const Wrapper = (await import('$lib/components/testing/PlaygroundPageWrapper.svelte')).default;
		render(Wrapper);

		await user.click(screen.getByRole('button', { name: 'Compare' }));

		const tablist = screen.getByRole('tablist', { name: 'Playground panes' });
		expect(tablist).toBeInTheDocument();

		const tabs = screen.getAllByRole('tab');
		expect(tabs).toHaveLength(2);

		const primaryTab = screen.getByRole('tab', { name: 'Primary' });
		expect(primaryTab).toHaveAttribute('aria-selected', 'true');
		expect(primaryTab).toHaveAttribute('aria-controls', 'pg-panel-primary');
		expect(primaryTab).toHaveAttribute('tabindex', '0');

		// The second tab is named after the comparison model (defaults to the
		// model that differs from the primary, i.e. Model B).
		const comparisonTab = screen.getByRole('tab', { name: 'Model B' });
		expect(comparisonTab).toHaveAttribute('aria-selected', 'false');
		expect(comparisonTab).toHaveAttribute('tabindex', '-1');

		// ArrowRight moves selection to the comparison tab.
		primaryTab.focus();
		await user.keyboard('{ArrowRight}');
		expect(comparisonTab).toHaveAttribute('aria-selected', 'true');
		expect(primaryTab).toHaveAttribute('aria-selected', 'false');

		const panels = screen.getAllByRole('tabpanel', { hidden: true });
		expect(panels.length).toBeGreaterThanOrEqual(2);
	});

	it('shows side-by-side panes (no tablist) in the wide layout when comparing', async () => {
		stubMatchMedia(false); // >= 1280px
		const user = userEvent.setup();
		const Wrapper = (await import('$lib/components/testing/PlaygroundPageWrapper.svelte')).default;
		render(Wrapper);

		await user.click(screen.getByRole('button', { name: 'Compare' }));

		expect(screen.queryByRole('tablist')).not.toBeInTheDocument();
		// Comparison pane section is present and labelled with the model name.
		expect(
			screen.getByRole('region', { name: 'Response from Model B (comparison)' })
		).toBeInTheDocument();
	});
});
