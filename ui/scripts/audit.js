#!/usr/bin/env node
// Final audit (PR14 §14.12): forbid raw `fetch(` outside the two sanctioned
// transport modules. Every other call site must go through the typed client.
//
//   - src/lib/api/client.ts  — the JSON apiFetch wrapper (timeouts, errors).
//   - src/lib/api/chat.ts    — streaming chat; legitimately needs raw fetch to
//                              read a `text/event-stream` body incrementally,
//                              which apiFetch (JSON-only) cannot do.
//
// Exits non-zero if a raw fetch appears anywhere else under src/.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve, relative } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(__dirname, '..', 'src');

const SANCTIONED = new Set(['lib/api/client.ts', 'lib/api/chat.ts']);
const FETCH_RE = /\bfetch\s*\(/;

function walk(dir, out) {
	for (const name of readdirSync(dir)) {
		const full = join(dir, name);
		if (statSync(full).isDirectory()) walk(full, out);
		// Production source only — tests legitimately stub/mock fetch.
		else if (/\.(ts|js|svelte)$/.test(name) && !/\.(test|spec)\.(ts|js)$/.test(name)) {
			out.push(full);
		}
	}
}

const files = [];
walk(SRC, files);

const offenders = [];
for (const file of files) {
	const rel = relative(SRC, file).replaceAll('\\', '/');
	if (SANCTIONED.has(rel)) continue;
	const lines = readFileSync(file, 'utf8').split('\n');
	lines.forEach((line, i) => {
		const trimmed = line.trim();
		// Skip comment lines.
		if (trimmed.startsWith('//') || trimmed.startsWith('*')) return;
		// Ignore prefetch/refetch/window.fetch-type identifiers; require a bare
		// `fetch(` not preceded by a word char, dot, or dash.
		if (FETCH_RE.test(line) && !/[\w.-]fetch\s*\(/.test(line)) {
			offenders.push(`${rel}:${i + 1}: ${trimmed}`);
		}
	});
}

if (offenders.length > 0) {
	console.error('✖ Raw fetch() found outside sanctioned modules (client.ts, chat.ts):');
	for (const o of offenders) console.error(`  - ${o}`);
	process.exit(1);
}

console.log('✓ No raw fetch() outside src/lib/api/client.ts and src/lib/api/chat.ts.');
