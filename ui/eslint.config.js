import prettier from 'eslint-config-prettier';
import path from 'node:path';
import { includeIgnoreFile } from '@eslint/compat';
import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import { defineConfig } from 'eslint/config';
import globals from 'globals';
import ts from 'typescript-eslint';
import svelteConfig from './svelte.config.js';

const gitignorePath = path.resolve(import.meta.dirname, '.gitignore');

// Enforces that `{@html ...}` is only ever used with the output of
// renderMarkdown() — the single sanitized source (FRONTEND_ARCHITECTURE.md §10.1).
// svelte-eslint-parser represents `{@html expr}` as a SvelteMustacheTag with
// kind === 'raw'.
const localPlugin = {
	rules: {
		'no-raw-html-without-sanitize': {
			meta: {
				type: 'problem',
				docs: { description: '{@html} is only allowed with renderMarkdown() output' },
				schema: [],
				messages: {
					unsafe:
						'{@html} may only render the output of renderMarkdown(); raw HTML from any other expression is forbidden.'
				}
			},
			create(context) {
				return {
					SvelteMustacheTag(node) {
						if (node.kind !== 'raw') return;
						const expr = node.expression;
						const isSanitized =
							expr &&
							expr.type === 'CallExpression' &&
							expr.callee.type === 'Identifier' &&
							expr.callee.name === 'renderMarkdown';
						if (!isSanitized) {
							context.report({ node, messageId: 'unsafe' });
						}
					}
				};
			}
		}
	}
};

export default defineConfig(
	includeIgnoreFile(gitignorePath),
	js.configs.recommended,
	ts.configs.recommended,
	svelte.configs.recommended,
	prettier,
	svelte.configs.prettier,
	{
		plugins: {
			local: localPlugin
		},
		languageOptions: { globals: { ...globals.browser, ...globals.node } },
		rules: {
			// typescript-eslint strongly recommend that you do not use the no-undef lint rule on TypeScript projects.
			// see: https://typescript-eslint.io/troubleshooting/faqs/eslint/#i-get-errors-from-the-no-undef-rule-about-global-variables-not-being-defined-even-though-there-are-no-typescript-errors
			'no-undef': 'off',
			'local/no-raw-html-without-sanitize': 'error',
			'svelte/no-navigation-without-resolve': 'off'
		}
	},
	{
		files: ['**/*.svelte', '**/*.svelte.ts', '**/*.svelte.js'],
		languageOptions: {
			parserOptions: {
				projectService: true,
				extraFileExtensions: ['.svelte'],
				parser: ts.parser,
				svelteConfig
			}
		}
	},
	{
		// Override or add rule settings here, such as:
		// 'svelte/button-has-type': 'error'
		rules: {}
	}
);
