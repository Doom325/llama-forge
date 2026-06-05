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
import type { BinaryScanResult, ServerConfigDraft, WizardStep } from '$lib/api/types';

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

	constructor(sessionId: string) {
		this.sessionId = sessionId;
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

/** Every present override field that is set must be valid. */
export function allFieldsValid(store: SetupWizardStore): boolean {
	for (const draft of store.userOverrides.values()) {
		if (draft.port !== undefined && draft.port <= 0) return false;
		if (draft.ctx_size !== undefined && draft.ctx_size <= 0) return false;
		if (draft.gpu_layers !== undefined && draft.gpu_layers < 0) return false;
		if (draft.parallel_slots !== undefined && draft.parallel_slots <= 0) return false;
		if (draft.alias !== undefined && draft.alias.trim() === '') return false;
	}
	return true;
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
		canAdvance: (_qc, store) => store.userOverrides.size > 0 && allFieldsValid(store),
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
