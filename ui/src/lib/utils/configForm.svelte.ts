import { beforeNavigate } from '$app/navigation';
import { ApiError } from '$lib/api/client';

/** Field-path → human-readable message. Empty object means "valid". */
export type ValidationErrors = Record<string, string>;

/** Structural value equality (used to derive `isDirty`, never a stored flag). */
export function deepEqual(a: unknown, b: unknown): boolean {
	if (a === b) return true;
	if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) {
		return false;
	}
	if (Array.isArray(a) !== Array.isArray(b)) return false;
	const aObj = a as Record<string, unknown>;
	const bObj = b as Record<string, unknown>;
	const aKeys = Object.keys(aObj);
	const bKeys = Object.keys(bObj);
	if (aKeys.length !== bKeys.length) return false;
	return aKeys.every((key) => deepEqual(aObj[key], bObj[key]));
}

export interface CreateConfigFormOptions<T> {
	/** The configuration as loaded from the server. */
	initial: T;
	/** ETag captured at load time, sent back as `If-Match` on save. */
	etag: string | null;
	/** Pure validator run on every edit. Return `{}` when valid. */
	validate: (data: T) => ValidationErrors;
	/** Persists the config; should reject with an {@link ApiError} on failure. */
	save: (args: { data: T; etag: string }) => Promise<unknown>;
}

export interface ConfigForm<T> {
	/** Editable working copy — bind form fields to this. */
	current: T;
	/** Immutable snapshot of the last-saved (or loaded) state. */
	readonly saved: T;
	/** `$derived` deep comparison of `current` vs `saved`. */
	readonly isDirty: boolean;
	readonly errors: ValidationErrors;
	readonly hasErrors: boolean;
	readonly isSubmitting: boolean;
	/** True after a 409 Conflict — another writer changed the config. */
	readonly conflict: boolean;
	/** Discard edits, restoring `current` to `saved`. */
	reset(): void;
	/** Re-initialise from fresh server data (used by the conflict Reload button). */
	rebase(next: T, etag: string | null): void;
	/** Validate, then persist. No-ops while submitting or when invalid. */
	submit(): Promise<void>;
}

/**
 * Reactive config-form state machine. Must be called during component
 * initialisation (it registers `$effect` and `beforeNavigate` guards).
 *
 * See FRONTEND_ARCHITECTURE.md §6.6.
 */
export function createConfigForm<T>(options: CreateConfigFormOptions<T>): ConfigForm<T> {
	const { validate, save } = options;

	let saved = $state(structuredClone(options.initial));
	let current = $state(structuredClone(options.initial));
	let etag = $state(options.etag);
	let isSubmitting = $state(false);
	let conflict = $state(false);

	const isDirty = $derived(!deepEqual(current, saved));
	const errors = $derived(validate(current));
	const hasErrors = $derived(Object.keys(errors).length > 0);

	// Browser-level guard: close tab / reload / external navigation while dirty.
	$effect(() => {
		const handler = (event: BeforeUnloadEvent) => {
			if (isDirty) {
				event.preventDefault();
				event.returnValue = '';
			}
		};
		window.addEventListener('beforeunload', handler);
		return () => window.removeEventListener('beforeunload', handler);
	});

	// SPA-level guard: SvelteKit client-side navigation while dirty.
	beforeNavigate((navigation) => {
		if (isDirty && !navigation.willUnload) {
			const leave = window.confirm('You have unsaved configuration changes. Leave without saving?');
			if (!leave) navigation.cancel();
		}
	});

	return {
		get current() {
			return current;
		},
		set current(next: T) {
			current = next;
		},
		get saved() {
			return saved;
		},
		get isDirty() {
			return isDirty;
		},
		get errors() {
			return errors;
		},
		get hasErrors() {
			return hasErrors;
		},
		get isSubmitting() {
			return isSubmitting;
		},
		get conflict() {
			return conflict;
		},
		reset() {
			current = $state.snapshot(saved) as T;
		},
		rebase(next: T, nextEtag: string | null) {
			saved = structuredClone(next);
			current = structuredClone(next);
			etag = nextEtag;
			conflict = false;
		},
		async submit() {
			if (hasErrors || isSubmitting) return;
			isSubmitting = true;
			conflict = false;
			const payload = $state.snapshot(current) as T;
			try {
				await save({ data: payload, etag: etag ?? '' });
				saved = structuredClone(payload);
			} catch (error) {
				if (error instanceof ApiError && error.status === 409) {
					conflict = true;
				}
				// Non-conflict errors are surfaced via the save mutation's onError toast.
			} finally {
				isSubmitting = false;
			}
		}
	};
}
