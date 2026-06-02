import { describe, it, expect } from 'vitest';
import { renderMarkdown } from './markdown';

describe('renderMarkdown', () => {
	it('strips <script> tags from untrusted markdown', () => {
		const html = renderMarkdown('<script>alert(1)</script>');
		expect(html).not.toContain('<script');
		expect(html.toLowerCase()).not.toContain('alert(1)</script>');
	});

	it('does not emit an <img> element (img is forbidden), dropping javascript: image sources', () => {
		const html = renderMarkdown('![x](javascript:alert(1))');
		expect(html).not.toContain('<img');
		expect(html.toLowerCase()).not.toContain('javascript:');
	});

	it('strips an <img onerror=...> XSS payload entirely', () => {
		const html = renderMarkdown('<img src=x onerror="alert(1)">');
		expect(html).not.toContain('<img');
		expect(html.toLowerCase()).not.toContain('onerror');
	});

	it('renders **bold** as <strong>', () => {
		const html = renderMarkdown('**bold**');
		expect(html).toContain('<strong>bold</strong>');
	});

	it('keeps http(s) links but drops javascript: hrefs', () => {
		const safe = renderMarkdown('[ok](https://example.com)');
		expect(safe).toContain('href="https://example.com"');

		const unsafe = renderMarkdown('[bad](javascript:alert(1))');
		expect(unsafe.toLowerCase()).not.toContain('javascript:');
		expect(unsafe).not.toContain('href="javascript:alert(1)"');
	});
});
