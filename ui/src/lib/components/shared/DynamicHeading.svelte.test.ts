import { render } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import DynamicHeadingWrapper from '../testing/DynamicHeadingWrapper.svelte';

describe('DynamicHeading', () => {
	it('renders h1 when level is 1', () => {
		const { container } = render(DynamicHeadingWrapper, { level: 1 });
		expect(container.querySelector('h1')).toBeInTheDocument();
		expect(container.querySelector('h1')).toHaveTextContent('Heading Content');
	});

	it('renders h6 when level is 6', () => {
		const { container } = render(DynamicHeadingWrapper, { level: 6 });
		expect(container.querySelector('h6')).toBeInTheDocument();
	});
});
