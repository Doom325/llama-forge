/**
 * Setup-wizard client state (08_SETUP_WIZARD.md Section 5.3).
 *
 * `SetupWizardStore` holds ONLY client-owned state that does not exist in any
 * API response — TanStack Query is the sole source of truth for server data
 * (hardware, binary scan, models, recommendations). The store is instantiated
 * in `SetupLayout` and provided via Svelte context, so it is scoped to the
 * wizard and garbage-collected when the wizard unmounts.
 */

import { getContext, setContext } from 'svelte';
import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import type { QueryClient } from '@tanstack/svelte-query';
import type {
	BinaryScanResult,
	ConfigRecommendation,
	ModelScanResult,
	RecommendationResult,
	ServerConfigDraft,
	WizardStep
} from '$lib/api/types';

/** Ordered wizard steps (also the URL order). */
export const WIZARD_STEPS: readonly WizardStep[] = [
	'welcome',
	'hardware',
	'binary',
	'models',
	'configure',
	'review'
] as const;

export class SetupWizardStore {
	/** Client-only model selections — not duplicated from any server response. */
	selectedModelIds = new SvelteSet<string>();
	/** Per-model field overrides applied on top of the server recommendation. */
	userOverrides = new SvelteMap<string, Partial<ServerConfigDraft>>();
	/** Chosen config save path. */
	savePath = $state<string>('');
	/** Correlation id sent as `X-Setup-Session-Id` on every wizard API call. */
	readonly sessionId: string;
	/**
	 * Reconfigure flow (Flow 4 / §13.8): the wizard was opened from the running
	 * app to add servers, so setup calls must send `X-Setup-Reconfigure` and the
	 * config save overwrites + merges the existing file.
	 */
	readonly reconfigure: boolean;

	constructor(sessionId: string, reconfigure = false) {
		this.sessionId = sessionId;
		this.reconfigure = reconfigure;
	}
}

// --- Svelte context plumbing -------------------------------------------------

const SETUP_STORE_KEY = Symbol('setup-wizard-store');

export function provideSetupStore(store: SetupWizardStore): void {
	setContext(SETUP_STORE_KEY, store);
}

export function useSetupStore(): SetupWizardStore {
	const store = getContext<SetupWizardStore | undefined>(SETUP_STORE_KEY);
	if (!store) {
		throw new Error('useSetupStore() called outside a SetupLayout context');
	}
	return store;
}

// --- Step state machine (Section 5.3) ---------------------------------------

/** Default alias for a model file (filename without the `.gguf` extension). */
function defaultAlias(filename: string): string {
	return filename.replace(/\.gguf$/i, '');
}

/**
 * Validate one selected model's *effective* config — the recommendation with the
 * user's override applied. Accepting the recommendation unedited is valid (goal
 * G1: get running without hand-editing); only a genuinely bad edit fails.
 */
function configValid(
	rec: ConfigRecommendation,
	override: Partial<ServerConfigDraft>,
	defaultAliasValue: string
): boolean {
	const port = override.port ?? rec.port;
	const ctxSize = override.ctx_size ?? rec.ctx_size;
	const gpuLayers = override.gpu_layers ?? rec.gpu_layers;
	const slots = override.parallel_slots ?? rec.parallel_slots;
	const alias = override.alias ?? defaultAliasValue;
	return (
		port > 0 &&
		port < 65536 &&
		ctxSize > 0 &&
		gpuLayers >= 0 &&
		slots > 0 &&
		alias.trim().length > 0
	);
}

/**
 * True when a recommendation exists for every selected model and each merged
 * (recommendation + override) config is valid. This is the configure-step
 * completion check — it does not require any edits (§5.3 / G1), only that the
 * resulting config is valid.
 */
export function allSelectedConfigsValid(qc: QueryClient, store: SetupWizardStore): boolean {
	const models = qc.getQueryData<ModelScanResult>(['setup', 'scan-models'])?.models ?? [];
	const recommendation = qc.getQueryData<RecommendationResult>(['setup', 'recommend']);
	if (!recommendation) return false;

	const selected = models.filter((m) => store.selectedModelIds.has(m.id));
	if (selected.length === 0) return false;

	return selected.every((model) => {
		const rec = recommendation.servers.find((s) => s.model_path === model.path)?.recommended;
		if (!rec) return false;
		return configValid(rec, store.userOverrides.get(model.id) ?? {}, defaultAlias(model.filename));
	});
}

export interface StepTransition {
	/** Whether the user may advance past this step. */
	canAdvance: (qc: QueryClient, store: SetupWizardStore) => boolean;
	/** Steps whose data is stale once this step is re-entered/changed. */
	invalidatesOnReentry: WizardStep[];
}

export const STEP_TRANSITIONS: Record<WizardStep, StepTransition> = {
	welcome: { canAdvance: () => true, invalidatesOnReentry: [] },
	hardware: {
		canAdvance: (qc) => qc.getQueryData(['setup', 'hardware']) != null,
		invalidatesOnReentry: ['configure', 'review']
	},
	binary: {
		canAdvance: (qc) => qc.getQueryData<BinaryScanResult>(['setup', 'scan-binary'])?.found === true,
		invalidatesOnReentry: ['configure', 'review']
	},
	models: {
		canAdvance: (_qc, store) => store.selectedModelIds.size > 0,
		invalidatesOnReentry: ['configure', 'review']
	},
	configure: {
		// Completion = a valid MERGED config for every selected model, NOT a
		// non-empty override set — accepting the recommendation as-is must advance
		// (goal G1: get running without hand-editing config).
		canAdvance: (qc, store) =>
			store.selectedModelIds.size > 0 && allSelectedConfigsValid(qc, store),
		invalidatesOnReentry: ['review']
	},
	review: { canAdvance: () => true, invalidatesOnReentry: [] }
};

/** TanStack query keys owned by each step (for invalidation). */
const STEP_QUERY_KEYS: Partial<Record<WizardStep, string[]>> = {
	hardware: ['setup', 'hardware'],
	binary: ['setup', 'scan-binary'],
	models: ['setup', 'scan-models'],
	configure: ['setup', 'recommend']
};

/**
 * Completed steps, *derived* by evaluating each step's guard — never stored
 * independently (Section 5.3). A step is complete iff its `canAdvance` holds.
 */
export function completedSteps(qc: QueryClient, store: SetupWizardStore): Set<WizardStep> {
	// Transient return value, not reactive state — a plain Set is correct here.
	// eslint-disable-next-line svelte/prefer-svelte-reactivity
	const done = new Set<WizardStep>();
	for (const step of WIZARD_STEPS) {
		if (STEP_TRANSITIONS[step].canAdvance(qc, store)) done.add(step);
	}
	return done;
}

/**
 * On re-entry to `target`, remove downstream server data so it re-fetches, and
 * clear dependent client-owned state (Section 5.3 — downstream invalidation).
 */
export function invalidateDownstream(
	qc: QueryClient,
	store: SetupWizardStore,
	target: WizardStep
): void {
	const downstream = STEP_TRANSITIONS[target].invalidatesOnReentry;
	for (const step of downstream) {
		const key = STEP_QUERY_KEYS[step];
		if (key) qc.removeQueries({ queryKey: key });
	}
	// Recommendations depend on hardware/binary/model selection; if any of those
	// is re-entered, the per-model overrides built on the old recommendation are
	// stale and must be dropped.
	if (downstream.includes('configure')) {
		store.userOverrides.clear();
	}
}
