#!/usr/bin/env node
// Bundle-size gate (PR14 §14.7/14.8). Sums the gzipped size of the EAGERLY
// loaded client JS — every entry chunk plus its transitive *static* imports —
// and enforces:
//   - a 250 KB gzipped aggregate budget, and
//   - a per-chunk drift threshold of 5 KB vs the committed baseline.
//
// Chunks reachable only via `dynamicImports` (e.g. echarts, highlight.js
// language grammars) are EXCLUDED: they are code-split and fetched on demand,
// so they are not part of the initial download the budget protects. Counting
// them would make a 250 KB budget meaningless (echarts alone is ~358 KB gz).
//
// Usage:
//   node scripts/check-bundle-size.js              # check against baseline
//   node scripts/check-bundle-size.js --write      # (re)generate the baseline
//
// Run `npm run build` first — this reads the Vite client manifest.

import { readFileSync, writeFileSync, existsSync, statSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const UI_ROOT = resolve(__dirname, '..');
const CLIENT_DIR = join(UI_ROOT, '.svelte-kit', 'output', 'client');
const MANIFEST_PATH = join(CLIENT_DIR, '.vite', 'manifest.json');
const BASELINE_PATH = join(UI_ROOT, 'bundle-baseline.json');

const AGGREGATE_BUDGET_BYTES = 250 * 1024; // 250 KB gzipped
const DRIFT_THRESHOLD_BYTES = 5 * 1024; // 5 KB per-chunk drift

const writeMode = process.argv.includes('--write');

function fail(message) {
	console.error(`\n✖ ${message}\n`);
	process.exit(1);
}

if (!existsSync(MANIFEST_PATH)) {
	fail(
		`Vite client manifest not found at:\n  ${MANIFEST_PATH}\n` +
			`Run \`npm run build\` first, then re-run this script.`
	);
}

// Logical, hash-free name for a manifest entry so per-chunk drift survives a
// rebuild (content hashes change every build). Prefers the stable chunk `name`,
// falling back to the manifest key with its hash stripped.
function logicalName(key, entry) {
	return entry.name ? `chunk:${entry.name}` : key.replace(/\.[a-z0-9]+\.js$/i, '.js');
}

// Collect the EAGER client JS graph: every manifest entry (`isEntry`) plus the
// chunks reachable through its static `imports` (transitively). `dynamicImports`
// edges are deliberately NOT followed, so lazily code-split chunks are excluded.
// Returns a Map of { stableLogicalName -> emitted .js file }.
function collectEagerJsChunks() {
	const manifest = JSON.parse(readFileSync(MANIFEST_PATH, 'utf8'));
	const chunks = new Map();
	const visited = new Set();

	const visit = (key) => {
		if (visited.has(key)) return;
		visited.add(key);
		const entry = manifest[key];
		if (!entry) return;
		if (entry.file && entry.file.endsWith('.js')) {
			chunks.set(logicalName(key, entry), entry.file);
		}
		for (const imp of entry.imports ?? []) visit(imp);
		// Intentionally skip entry.dynamicImports — those are on-demand chunks.
	};

	for (const [key, entry] of Object.entries(manifest)) {
		if (entry.isEntry) visit(key);
	}
	return chunks;
}

function gzippedSize(relPath) {
	const abs = join(CLIENT_DIR, relPath);
	if (!existsSync(abs) || !statSync(abs).isFile()) return 0;
	return gzipSync(readFileSync(abs)).length;
}

function kb(bytes) {
	return (bytes / 1024).toFixed(2);
}

const jsChunks = collectEagerJsChunks();
if (jsChunks.size === 0) fail('No client JS chunks found — did the build succeed?');

const sizes = {}; // logicalKey -> gzipped bytes
let total = 0;
for (const [logical, file] of jsChunks) {
	const size = gzippedSize(file);
	sizes[logical] = size;
	total += size;
}

// --- Write-baseline mode ---------------------------------------------------
if (writeMode) {
	const baseline = {
		generatedAt: new Date().toISOString(),
		aggregateBudgetBytes: AGGREGATE_BUDGET_BYTES,
		driftThresholdBytes: DRIFT_THRESHOLD_BYTES,
		totalGzipBytes: total,
		chunks: sizes
	};
	writeFileSync(BASELINE_PATH, JSON.stringify(baseline, null, 2) + '\n');
	console.log(`✓ Wrote baseline (${jsChunks.size} chunks, ${kb(total)} KB gzipped) to`);
	console.log(`  ${BASELINE_PATH}`);
	process.exit(0);
}

// --- Check mode ------------------------------------------------------------
const baseline = existsSync(BASELINE_PATH) ? JSON.parse(readFileSync(BASELINE_PATH, 'utf8')) : null;
const baselineChunks = baseline?.chunks ?? {};

const rows = [...jsChunks.keys()].sort().map((logical) => {
	const cur = sizes[logical];
	const base = baselineChunks[logical] ?? null;
	const drift = base === null ? null : cur - base;
	return { file: logical, cur, base, drift };
});

// Print a readable table.
const nameWidth = Math.max(...rows.map((r) => r.file.length), 'Chunk'.length);
const pad = (s, w) => String(s).padEnd(w);
const padL = (s, w) => String(s).padStart(w);
console.log('\nBundle size report (gzipped):');
console.log(
	`${pad('Chunk', nameWidth)}  ${padL('Current', 10)}  ${padL('Baseline', 10)}  ${padL('Drift', 10)}`
);
console.log('-'.repeat(nameWidth + 38));
for (const r of rows) {
	console.log(
		`${pad(r.file, nameWidth)}  ${padL(kb(r.cur) + ' KB', 10)}  ` +
			`${padL(r.base === null ? '—' : kb(r.base) + ' KB', 10)}  ` +
			`${padL(r.drift === null ? 'new' : (r.drift >= 0 ? '+' : '') + kb(r.drift) + ' KB', 10)}`
	);
}
console.log('-'.repeat(nameWidth + 38));
console.log(
	`${pad('TOTAL', nameWidth)}  ${padL(kb(total) + ' KB', 10)}  ` +
		`${padL(baseline ? kb(baseline.totalGzipBytes) + ' KB' : '—', 10)}`
);
console.log(`Aggregate budget: ${kb(AGGREGATE_BUDGET_BYTES)} KB\n`);

const failures = [];

if (total > AGGREGATE_BUDGET_BYTES) {
	failures.push(
		`Aggregate gzipped JS (${kb(total)} KB) exceeds budget (${kb(AGGREGATE_BUDGET_BYTES)} KB).`
	);
}

if (baseline) {
	for (const r of rows) {
		if (r.drift !== null && r.drift > DRIFT_THRESHOLD_BYTES) {
			failures.push(
				`Chunk "${r.file}" grew by ${kb(r.drift)} KB (> ${kb(DRIFT_THRESHOLD_BYTES)} KB drift threshold).`
			);
		}
	}
} else {
	console.warn(
		'⚠ No baseline found — skipping per-chunk drift check. Run with --write to create one.'
	);
}

if (failures.length > 0) {
	console.error('✖ Bundle-size check failed:');
	for (const f of failures) console.error(`  - ${f}`);
	console.error('');
	process.exit(1);
}

console.log('✓ Bundle-size check passed.\n');
