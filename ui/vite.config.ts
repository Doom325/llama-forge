import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import { sveltekit } from '@sveltejs/kit/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import viteCompression from 'vite-plugin-compression';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit(), svelteTesting(), viteCompression()],
	// Emit .vite/manifest.json so the bundle-size CI check (PR14) can map routes
	// to chunks and enforce per-route budgets.
	build: { manifest: true },
	// Dev-only proxy so `npm run dev` on :5173 can reach the orchestrator on
	// :8000. The API client uses absolute paths (`/api/...`, `/v1/...`); SSE
	// lives at `/api/events` and is caught by the `/api` rule. This block is
	// ignored by `vite build` — in production the Rust transport serves the UI
	// and API from the same origin, so no CSP `connect-src` adjustment is
	// needed there.
	server: {
		proxy: {
			'/api': { target: 'http://localhost:8000', changeOrigin: true, ws: true },
			'/v1': { target: 'http://localhost:8000', changeOrigin: true }
		}
	},
	test: {
		expect: { requireAssertions: true },
		environment: 'jsdom',
		coverage: {
			thresholds: {
				statements: 80,
				branches: 75,
				functions: 80,
				lines: 80
			}
		},
		projects: [
			{
				extends: './vite.config.ts',
				test: {
					name: 'client',
					environment: 'jsdom',
					setupFiles: ['./vitest-setup-client.ts'],
					include: ['src/**/*.svelte.{test,spec}.{js,ts}']
				}
			},
			{
				extends: './vite.config.ts',
				test: {
					name: 'server',
					environment: 'node',
					include: ['src/**/*.{test,spec}.{js,ts}'],
					exclude: ['src/**/*.svelte.{test,spec}.{js,ts}']
				}
			}
		]
	}
});
