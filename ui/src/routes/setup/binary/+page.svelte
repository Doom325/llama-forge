<script lang="ts">
	// BinaryStep orchestrator (08_SETUP_WIZARD.md §2.1/4/6.2): scans for an
	// existing llama-server, and otherwise drives a streamed build. The build
	// lifecycle is owned here; the SSE→state transitions live in the pure,
	// separately-tested `applyBuildEvent` reducer.
	import {
		createBinaryScanMutation,
		createHardwareQuery,
		createBuildMutation
	} from '$lib/api/queries';
	import { queryClient } from '$lib/api/queryClient';
	import { apiFetch } from '$lib/api/client';
	import { useSetupStore } from '$lib/stores/setup.svelte';
	import { applyBuildEvent, type BuildEventInput } from '$lib/components/setup/buildStatus';
	import BuildConfigurator from '$lib/components/setup/BuildConfigurator.svelte';
	import BuildProgress from '$lib/components/setup/BuildProgress.svelte';
	import Skeleton from '$lib/components/shared/Skeleton.svelte';
	import type { BuildPhase, BuildStatus, SetupBackend } from '$lib/api/types';

	const store = useSetupStore();
	const scan = createBinaryScanMutation(store.sessionId, store.reconfigure);
	const build = createBuildMutation(store.sessionId, store.reconfigure);
	const hw = createHardwareQuery(store.sessionId, store.reconfigure);

	let hasScanned = $state(false);
	let manualPath = $state('');
	let buildStatus = $state<BuildStatus>({ status: 'idle' });
	let buildId = $state<string | null>(null);
	let lastRequest = $state<{ backend: SetupBackend; install_dir: string; jobs: number } | null>(
		null
	);

	const detectedBackend = $derived(hw.data?.recommended_backend ?? 'cpu');

	// Auto-scan standard locations once on entry.
	$effect(() => {
		if (!hasScanned) {
			hasScanned = true;
			scan.mutate([]);
		}
	});

	/** Map a parsed SSE event into the typed reducer input (avoids `any`). */
	function toEventInput(name: string, raw: unknown): BuildEventInput | null {
		const d = raw as Record<string, unknown>;
		switch (name) {
			case 'phase':
				return {
					event: 'phase',
					data: { phase: d.phase as BuildPhase, message: String(d.message ?? '') }
				};
			case 'progress':
				return {
					event: 'progress',
					data: { phase: d.phase as BuildPhase, percent: Number(d.percent) }
				};
			case 'log':
				return { event: 'log', data: { lines: (d.lines as string[]) ?? [] } };
			case 'complete':
				return {
					event: 'complete',
					data: {
						binary_path: String(d.binary_path),
						duration_secs: Number(d.duration_secs),
						replaced_existing: Boolean(d.replaced_existing),
						backup_path: (d.backup_path as string | null) ?? null,
						log_path: d.log_path as string | undefined
					}
				};
			case 'failed':
				return {
					event: 'failed',
					data: {
						phase: d.phase as BuildPhase,
						error: String(d.error),
						last_lines: (d.last_lines as string[]) ?? [],
						log_path: d.log_path as string | undefined
					}
				};
			default:
				return null;
		}
	}

	// Stream build progress while a build is running. Reconnects with exponential
	// backoff (1s→8s, 3 attempts); the EventSource is closed on cleanup.
	$effect(() => {
		const id = buildId;
		if (!id) return;

		let source: EventSource | null = null;
		let attempts = 0;
		let retryTimer: ReturnType<typeof setTimeout> | undefined;

		const handle = (name: string) => (event: MessageEvent) => {
			// An event arrived → the connection is healthy; reset the
			// consecutive-failure counter (design: 3 *consecutive* disconnects).
			attempts = 0;
			// A malformed event must not break the stream — skip it and keep going.
			let parsed: unknown;
			try {
				parsed = JSON.parse(event.data);
			} catch {
				return;
			}
			const input = toEventInput(name, parsed);
			if (!input) return;
			buildStatus = applyBuildEvent(buildStatus, id, input);
			if (input.event === 'complete') {
				// Reflect the freshly-built binary so the step's guard passes.
				queryClient.setQueryData(['setup', 'scan-binary'], {
					found: true,
					path: input.data.binary_path,
					binary_version: '(freshly built)',
					backends: []
				});
				source?.close();
			} else if (input.event === 'failed') {
				source?.close();
			}
		};

		const open = () => {
			// The backend replays the FULL backlog on every connection (no
			// per-client cursor), so rebuild state from scratch on each (re)connect
			// — otherwise the replayed `log` events would double the log lines.
			buildStatus = {
				status: 'running',
				build_id: id,
				phase: 'cloning',
				percent: null,
				logLines: []
			};
			source = new EventSource(`/api/setup/build/${id}/stream`);
			for (const name of ['phase', 'progress', 'log', 'complete', 'failed']) {
				source.addEventListener(name, handle(name) as EventListener);
			}
			source.onerror = () => {
				source?.close();
				if (buildStatus.status === 'running' && attempts < 3) {
					attempts += 1;
					retryTimer = setTimeout(open, Math.min(8000, 1000 * 2 ** (attempts - 1)));
				}
			};
		};
		open();

		return () => {
			source?.close();
			if (retryTimer) clearTimeout(retryTimer);
		};
	});

	async function startBuild(request: { backend: SetupBackend; install_dir: string; jobs: number }) {
		lastRequest = request;
		const result = await build.mutateAsync(request);
		buildId = result.build_id;
		buildStatus = {
			status: 'running',
			build_id: result.build_id,
			phase: 'cloning',
			percent: null,
			logLines: []
		};
	}

	function resetBuild() {
		buildId = null;
		buildStatus = { status: 'idle' };
	}

	async function cancelBuild() {
		const id = buildId;
		if (id) {
			await apiFetch(`/api/setup/build/${id}`, { method: 'DELETE' }).catch(() => {});
		}
		resetBuild();
	}

	const fieldClass =
		'w-full rounded border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none';
</script>

<h1 tabindex="-1" class="text-2xl font-semibold text-[var(--color-text-primary)]">llama-server</h1>
<p class="mt-2 text-[var(--color-text-secondary)]">
	Locating the <code>llama-server</code> binary, or building it from source.
</p>

<div class="mt-6 space-y-4">
	{#if buildStatus.status !== 'idle'}
		<BuildProgress
			status={buildStatus}
			oncancel={cancelBuild}
			onretry={() => lastRequest && startBuild(lastRequest)}
			onchangebackend={resetBuild}
			onskip={resetBuild}
		/>
	{:else if scan.isPending}
		<p class="text-sm text-[var(--color-text-secondary)]">Scanning standard locations…</p>
		<Skeleton height="3rem" />
	{:else if scan.data?.found}
		<div class="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-4">
			<p class="text-sm font-medium text-[var(--color-text-primary)]">Found llama-server</p>
			<p class="mt-1 font-mono text-xs text-[var(--color-text-secondary)]">{scan.data.path}</p>
			<p class="mt-1 text-xs text-[var(--color-text-secondary)]">
				Version {scan.data.binary_version}
			</p>
		</div>
	{:else}
		<div class="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-4">
			<p class="text-sm font-medium text-[var(--color-text-primary)]">
				No llama-server found on this system.
			</p>
			<p class="mt-1 text-sm text-[var(--color-text-secondary)]">
				Build it from source, or point to an existing binary directory.
			</p>
		</div>

		<BuildConfigurator detectedBackend={detectedBackend as SetupBackend} onstart={startBuild} />

		<div class="space-y-2">
			<label for="binary-path" class="text-sm font-medium text-[var(--color-text-primary)]">
				…or search a specific directory
			</label>
			<div class="flex gap-2">
				<input
					id="binary-path"
					bind:value={manualPath}
					class={fieldClass}
					placeholder="/opt/llama.cpp/build/bin"
				/>
				<button
					type="button"
					onclick={() => scan.mutate(manualPath ? [manualPath] : [])}
					class="rounded border border-[var(--color-border)] px-4 py-2 text-sm font-medium text-[var(--color-text-primary)] focus:ring-2 focus:ring-[var(--color-accent-fill)] focus:outline-none"
				>
					Search
				</button>
			</div>
		</div>
	{/if}
</div>
