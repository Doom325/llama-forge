import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	compilerOptions: {
		// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
		runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true)
	},
	kit: {
		adapter: adapter({
			fallback: 'index.html'
		}),
		paths: {
			base: '/ui'
		},
		// SvelteKit owns the script/style CSP: in `hash` mode it injects a
		// <meta> CSP whose script-src includes sha256 hashes of its own inline
		// bootstrap, so the SPA hydrates under a strict policy (no 'unsafe-inline'
		// for scripts). The Rust server must NOT also send a script-src header —
		// CSPs are enforced as an intersection, so a hash-less server script-src
		// would re-block these inline scripts (see transport/ui.rs).
		csp: {
			mode: 'hash',
			directives: {
				'default-src': ['self'],
				'script-src': ['self'],
				'style-src': ['self', 'unsafe-inline'],
				'img-src': ['self', 'data:', 'blob:'],
				'connect-src': ['self'],
				'font-src': ['self'],
				'object-src': ['none'],
				'base-uri': ['self'],
				'form-action': ['self']
			}
		}
	}
};

export default config;
