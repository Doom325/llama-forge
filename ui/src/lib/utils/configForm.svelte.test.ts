import { describe, it, expect, vi } from 'vitest';
import { ApiError } from '$lib/api/client';

// `createConfigForm` registers `beforeNavigate`; stub it so it is a no-op in
// the test environment (jsdom has no SvelteKit navigation lifecycle).
vi.mock('$app/navigation', () => ({
	beforeNavigate: () => {}
}));

import { deepEqual, createConfigForm } from './configForm.svelte';

describe('deepEqual', () => {
	it('treats identical primitives as equal', () => {
		expect(deepEqual(1, 1)).toBe(true);
		expect(deepEqual('a', 'a')).toBe(true);
		expect(deepEqual(true, true)).toBe(true);
		expect(deepEqual(null, null)).toBe(true);
	});

	it('treats differing primitives as unequal', () => {
		expect(deepEqual(1, 2)).toBe(false);
		expect(deepEqual('a', 'b')).toBe(false);
		expect(deepEqual(null, undefined)).toBe(false);
		expect(deepEqual(0, false)).toBe(false);
	});

	it('compares nested objects structurally', () => {
		expect(deepEqual({ a: 1, b: { c: 2 } }, { a: 1, b: { c: 2 } })).toBe(true);
		expect(deepEqual({ a: 1, b: { c: 2 } }, { a: 1, b: { c: 3 } })).toBe(false);
		expect(deepEqual({ a: 1 }, { a: 1, b: 2 })).toBe(false);
	});

	it('compares arrays element-wise and distinguishes arrays from objects', () => {
		expect(deepEqual([1, 2, 3], [1, 2, 3])).toBe(true);
		expect(deepEqual([1, 2, 3], [1, 2])).toBe(false);
		expect(deepEqual([1, 2], { 0: 1, 1: 2 })).toBe(false);
	});

	it('handles arrays of objects', () => {
		const a = [{ id: 'x', n: 1 }];
		const b = [{ id: 'x', n: 1 }];
		expect(deepEqual(a, b)).toBe(true);
		expect(deepEqual(a, [{ id: 'x', n: 2 }])).toBe(false);
	});
});

interface Sample {
	name: string;
	count: number;
}

describe('createConfigForm', () => {
	function setup(overrides?: {
		validate?: (d: Sample) => Record<string, string>;
		save?: (args: { data: Sample; etag: string }) => Promise<unknown>;
	}) {
		const initial: Sample = { name: 'one', count: 1 };
		const validate = overrides?.validate ?? (() => ({}));
		const save = overrides?.save ?? (() => Promise.resolve());
		return createConfigForm<Sample>({ initial, etag: 'v1', validate, save });
	}

	it('flips isDirty when current changes and clears it on reset', () => {
		const cleanup = $effect.root(() => {
			const form = setup();
			expect(form.isDirty).toBe(false);
			form.current.name = 'two';
			expect(form.isDirty).toBe(true);
			form.reset();
			expect(form.isDirty).toBe(false);
			expect(form.current.name).toBe('one');
		});
		cleanup();
	});

	it('populates errors and hasErrors from a failing validator', () => {
		const cleanup = $effect.root(() => {
			const form = setup({
				validate: (d) => (d.count < 1 ? { count: 'too small' } : {})
			});
			expect(form.hasErrors).toBe(false);
			form.current.count = 0;
			expect(form.hasErrors).toBe(true);
			expect(form.errors.count).toBe('too small');
		});
		cleanup();
	});

	it('sets conflict=true when save rejects with a 409 ApiError', async () => {
		let form!: ReturnType<typeof setup>;
		const cleanup = $effect.root(() => {
			form = setup({
				save: () => Promise.reject(new ApiError(409, 'conflict', false))
			});
		});
		await form.submit();
		expect(form.conflict).toBe(true);
		cleanup();
	});

	it('updates saved on a resolving submit so isDirty becomes false', async () => {
		let form!: ReturnType<typeof setup>;
		const cleanup = $effect.root(() => {
			form = setup();
			form.current.name = 'changed';
			expect(form.isDirty).toBe(true);
		});
		await form.submit();
		expect(form.isDirty).toBe(false);
		expect(form.saved.name).toBe('changed');
		cleanup();
	});

	it('rebase resets dirty and conflict from fresh server data', async () => {
		let form!: ReturnType<typeof setup>;
		const cleanup = $effect.root(() => {
			form = setup({
				save: () => Promise.reject(new ApiError(409, 'conflict', false))
			});
			form.current.name = 'dirty';
		});
		await form.submit();
		expect(form.conflict).toBe(true);
		form.rebase({ name: 'server', count: 9 }, 'v2');
		expect(form.conflict).toBe(false);
		expect(form.isDirty).toBe(false);
		expect(form.current.name).toBe('server');
		cleanup();
	});
});
