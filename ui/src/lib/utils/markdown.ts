import { marked, type Tokens } from 'marked';
import DOMPurify from 'dompurify';
import type { HLJSApi } from 'highlight.js';

// The ONLY place {@html} content is produced. LLM output is untrusted (prompt
// injection from upstream data), so the chain is explicit and unconditional:
// marked → DOMPurify → {@html}. See FRONTEND_ARCHITECTURE.md §10.1.
const DOMPURIFY_CONFIG = {
	ALLOWED_TAGS: [
		'p',
		'br',
		'strong',
		'em',
		'code',
		'pre',
		'ul',
		'ol',
		'li',
		'h1',
		'h2',
		'h3',
		'h4',
		'h5',
		'h6',
		'blockquote',
		'a',
		'table',
		'thead',
		'tbody',
		'tr',
		'th',
		'td',
		'hr',
		'del',
		'sup',
		'sub',
		'span' // syntax-highlighting classes only
	],
	ALLOWED_ATTR: [
		'href', // on <a> only — protocol-restricted below
		'class' // syntax-highlighting CSS classes
	],
	FORBID_TAGS: [
		'style',
		'script',
		'iframe',
		'form',
		'input',
		'img',
		'svg',
		'math',
		'object',
		'embed'
	],
	// DOMPurify strips all on* handlers by default; listed explicitly for clarity.
	FORBID_ATTR: ['style', 'onerror', 'onload', 'onclick'],
	ALLOW_DATA_ATTR: false,
	ALLOWED_URI_REGEXP: /^(?:https?|mailto):/i
};

// highlight.js is large, so it is loaded lazily — only once the playground
// mounts and calls loadHighlighter(). Until then, code blocks render as plain
// (escaped) text. The selected core grammars keep the lazy chunk small (~15 KB).
let hljs: HLJSApi | null = null;
let highlighterPromise: Promise<void> | null = null;

export function loadHighlighter(): Promise<void> {
	if (!highlighterPromise) {
		highlighterPromise = (async () => {
			const core = (await import('highlight.js/lib/core')).default;
			const [javascript, typescript, python, json, yaml, bash] = await Promise.all([
				import('highlight.js/lib/languages/javascript'),
				import('highlight.js/lib/languages/typescript'),
				import('highlight.js/lib/languages/python'),
				import('highlight.js/lib/languages/json'),
				import('highlight.js/lib/languages/yaml'),
				import('highlight.js/lib/languages/bash')
			]);
			core.registerLanguage('javascript', javascript.default);
			core.registerLanguage('typescript', typescript.default);
			core.registerLanguage('python', python.default);
			core.registerLanguage('json', json.default);
			core.registerLanguage('yaml', yaml.default);
			core.registerLanguage('bash', bash.default);
			hljs = core;
		})();
	}
	return highlighterPromise;
}

function escapeHtml(text: string): string {
	return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Custom code renderer: highlight.js (text-only output) when loaded and the
// language is known; otherwise escaped plain text. Output is sanitized anyway.
marked.use({
	renderer: {
		code(token: Tokens.Code): string {
			const lang = (token.lang ?? '').trim().split(/\s+/)[0];
			if (hljs && lang && hljs.getLanguage(lang)) {
				const highlighted = hljs.highlight(token.text, { language: lang }).value;
				return `<pre><code class="hljs language-${lang}">${highlighted}</code></pre>`;
			}
			return `<pre><code class="hljs">${escapeHtml(token.text)}</code></pre>`;
		}
	}
});

/**
 * Render untrusted markdown to sanitized HTML. The return value is the ONLY
 * string that may be passed to `{@html}` (enforced by the
 * `local/no-raw-html-without-sanitize` ESLint rule).
 */
export function renderMarkdown(untrustedMarkdown: string): string {
	const rawHtml = marked.parse(untrustedMarkdown, { async: false }) as string;
	return DOMPurify.sanitize(rawHtml, DOMPURIFY_CONFIG);
}
