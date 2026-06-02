import { render } from '@testing-library/svelte';
import { describe, it, expect } from 'vitest';
import MessageBubble from './MessageBubble.svelte';
import type { Message } from '$lib/api/types';

describe('MessageBubble', () => {
	it('renders assistant markdown as sanitized HTML', () => {
		const message: Message = {
			role: 'assistant',
			content: 'this is **bold**',
			timestamp: 1
		};
		const { container } = render(MessageBubble, { message });

		const strong = container.querySelector('strong');
		expect(strong).not.toBeNull();
		expect(strong?.textContent).toBe('bold');
	});

	it('does not inject a <script> element from assistant content', () => {
		const message: Message = {
			role: 'assistant',
			content: 'hi <script>window.__pwned = true</script>',
			timestamp: 1
		};
		const { container } = render(MessageBubble, { message });

		expect(container.querySelector('script')).toBeNull();
	});

	it('escapes user content as plain text (no HTML injection)', () => {
		const message: Message = {
			role: 'user',
			content: '<b>not bold</b>',
			timestamp: 1
		};
		const { container } = render(MessageBubble, { message });

		// User content is rendered as text, so no <b> element is created.
		expect(container.querySelector('b')).toBeNull();
		expect(container.textContent).toContain('<b>not bold</b>');
	});
});
