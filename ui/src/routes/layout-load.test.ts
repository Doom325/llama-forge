import { describe, it, expect, vi, beforeEach } from 'vitest';
import { isRedirect } from '@sveltejs/kit';

// The root layout `load` is a plain function, so we exercise it directly (node
// project). `browser` is toggled per test; `base` is pinned to the app's '/ui';
// fetchSetupStatus is stubbed so no real network/client code is pulled in.
const mocks = vi.hoisted(() => ({ browser: true, fetchSetupStatus: vi.fn() }));

vi.mock('$app/environment', () => ({
	get browser() {
		return mocks.browser;
	}
}));
vi.mock('$app/paths', () => ({ base: '/ui' }));
vi.mock('$lib/api/queries', () => ({
	fetchSetupStatus: () => mocks.fetchSetupStatus()
}));

type LoadFn = (event: { url: URL }) => Promise<unknown>;
const ev = (pathname: string): { url: URL } => ({ url: new URL(`http://localhost${pathname}`) });

// The `setupConfirmed` flag is module-level state, so each test gets a fresh module.
async function freshLoad(): Promise<LoadFn> {
	vi.resetModules();
	return (await import('./+layout')).load as LoadFn;
}

beforeEach(() => {
	mocks.browser = true;
	mocks.fetchSetupStatus = vi.fn();
});

describe('root layout load — first-run redirect (#288)', () => {
	it('redirects to /ui/setup when the backend reports setup_required', async () => {
		mocks.fetchSetupStatus = vi.fn().mockResolvedValue({ setup_required: true, version: 1 });
		const load = await freshLoad();

		const thrown = await load(ev('/ui')).then(
			() => null,
			(e) => e
		);
		expect(isRedirect(thrown)).toBe(true);
		expect((thrown as { status: number }).status).toBe(307);
		expect((thrown as { location: string }).location).toBe('/ui/setup');
	});

	it('loads the dashboard (no redirect) in running mode', async () => {
		mocks.fetchSetupStatus = vi.fn().mockResolvedValue({ setup_required: false, version: 1 });
		const load = await freshLoad();
		await expect(load(ev('/ui'))).resolves.toEqual({});
	});

	it('never probes or redirects on the wizard subtree (no loop)', async () => {
		const probe = vi.fn().mockResolvedValue({ setup_required: true, version: 1 });
		mocks.fetchSetupStatus = probe;
		const load = await freshLoad();

		await expect(load(ev('/ui/setup'))).resolves.toEqual({});
		await expect(load(ev('/ui/setup/hardware'))).resolves.toEqual({});
		expect(probe).not.toHaveBeenCalled();
	});

	it('falls through (no redirect) when the status probe fails', async () => {
		mocks.fetchSetupStatus = vi.fn().mockRejectedValue(new Error('backend down'));
		const load = await freshLoad();
		await expect(load(ev('/ui'))).resolves.toEqual({});
	});

	it('skips the probe entirely during SSR (browser=false)', async () => {
		mocks.browser = false;
		const probe = vi.fn();
		mocks.fetchSetupStatus = probe;
		const load = await freshLoad();
		await expect(load(ev('/ui'))).resolves.toEqual({});
		expect(probe).not.toHaveBeenCalled();
	});

	it('stops re-probing /api/status once running mode is confirmed', async () => {
		const probe = vi.fn().mockResolvedValue({ setup_required: false, version: 1 });
		mocks.fetchSetupStatus = probe;
		const load = await freshLoad();

		await load(ev('/ui'));
		await load(ev('/ui/servers'));
		await load(ev('/ui/playground'));
		expect(probe).toHaveBeenCalledTimes(1);
	});
});
