// Lighthouse CI config (PR14 §14.10). Runs in CI only — do not run locally.
// `autorun` builds nothing itself; the workflow runs `npm run build && npm run
// preview` (the SvelteKit static preview serves the app at /ui on :4173) before
// invoking `@lhci/cli autorun`.
export default {
	ci: {
		collect: {
			// Preview server (vite preview) serves the static build at /ui.
			startServerCommand: 'npm run preview',
			startServerReadyPattern: 'Local:',
			startServerReadyTimeout: 60000,
			url: [
				'http://localhost:4173/ui',
				'http://localhost:4173/ui/explorer',
				'http://localhost:4173/ui/playground'
			],
			numberOfRuns: 1,
			settings: {
				// Desktop-class run; the app is an operator console, not mobile-first.
				preset: 'desktop'
			}
		},
		assert: {
			assertions: {
				'categories:performance': ['error', { minScore: 0.9 }],
				'categories:accessibility': ['error', { minScore: 0.95 }],
				'categories:best-practices': ['error', { minScore: 0.9 }]
			}
		},
		upload: {
			target: 'temporary-public-storage'
		}
	}
};
