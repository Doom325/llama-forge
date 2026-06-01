import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';
import { sveltekit } from '@sveltejs/kit/vite';
import viteCompression from 'vite-plugin-compression';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit(), viteCompression()],
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
