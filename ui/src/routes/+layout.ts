import { browser } from '$app/environment';
import { redirect } from '@sveltejs/kit';
import { base } from '$app/paths';
import { fetchSetupStatus } from '$lib/api/queries';
import type { LayoutLoad } from './$types';

export const ssr = false;

// First-run guard (issue #288): if the backend reports setup is still required,
// route any non-wizard route to /ui/setup so the dashboard isn't shown with every
// panel 503-ing behind the setup-mode guard. This is the forward half of the
// symmetry — the reverse (leaving the wizard once setup_required flips to false)
// lives in setup/review/+page.svelte.
//
// The /setup subtree is excluded so the wizard itself can't redirect-loop. A
// failed probe falls through without redirecting, so an unreachable backend
// surfaces through the dashboard's own panels rather than trapping the user on a
// blank wizard. Once we've confirmed setup is NOT required, the probe is skipped
// for the rest of the session — the running dashboard needn't re-hit /api/status
// on every client-side navigation (this load re-runs whenever the URL changes).
let setupConfirmed = false;

export const load: LayoutLoad = async ({ url }) => {
	if (!browser || setupConfirmed) return {};
	if (url.pathname.startsWith(`${base}/setup`)) return {};

	let setupRequired = false;
	try {
		setupRequired = (await fetchSetupStatus()).setup_required;
		// Running mode confirmed — stop probing for the rest of the session.
		setupConfirmed = !setupRequired;
	} catch {
		// Backend unreachable / errored: don't redirect (that would strand the user
		// on the wizard). Let the dashboard render its own error UI, and re-probe on
		// the next navigation.
	}

	// The redirect lives outside the try so a thrown redirect can never be confused
	// with — or swallowed alongside — an ApiError (apiFetch throws ApiError, which
	// also carries a numeric `status`).
	if (setupRequired) redirect(307, `${base}/setup`);
	return {};
};
