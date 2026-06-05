// Pure reducer mapping streamed build SSE events onto a BuildStatus (Section
// 11.4). Kept framework-free + side-effect-free so the BinaryStep orchestrator
// just feeds parsed events in, and so each transition is unit-testable.

import type { BuildPhase, BuildStatus } from '$lib/api/types';

/** Parsed SSE event payloads (the `data` of each named build event). */
export type BuildEventInput =
	| { event: 'phase'; data: { phase: BuildPhase; message: string } }
	| { event: 'progress'; data: { phase: BuildPhase; percent: number } }
	| { event: 'log'; data: { lines: string[] } }
	| {
			event: 'complete';
			data: {
				binary_path: string;
				duration_secs: number;
				replaced_existing: boolean;
				backup_path?: string | null;
				log_path?: string;
			};
	  }
	| {
			event: 'failed';
			data: { phase: BuildPhase; error: string; last_lines: string[]; log_path?: string };
	  };

function asRunning(
	prev: BuildStatus,
	buildId: string
): Extract<BuildStatus, { status: 'running' }> {
	if (prev.status === 'running') return prev;
	return { status: 'running', build_id: buildId, phase: 'cloning', percent: null, logLines: [] };
}

/**
 * Fold one build event into the current status. `log` events accumulate
 * `logLines`; terminal events (`complete`/`failed`) replace the status.
 */
export function applyBuildEvent(
	prev: BuildStatus,
	buildId: string,
	input: BuildEventInput
): BuildStatus {
	switch (input.event) {
		case 'phase': {
			const running = asRunning(prev, buildId);
			return { ...running, phase: input.data.phase };
		}
		case 'progress': {
			const running = asRunning(prev, buildId);
			return { ...running, phase: input.data.phase, percent: input.data.percent };
		}
		case 'log': {
			const running = asRunning(prev, buildId);
			return { ...running, logLines: [...running.logLines, ...input.data.lines] };
		}
		case 'complete':
			return {
				status: 'complete',
				build_id: buildId,
				binary_path: input.data.binary_path,
				duration_secs: input.data.duration_secs,
				replaced_existing: input.data.replaced_existing,
				backup_path: input.data.backup_path ?? null,
				// `exactOptionalPropertyTypes`: omit rather than set undefined.
				...(input.data.log_path !== undefined ? { log_path: input.data.log_path } : {})
			};
		case 'failed':
			return {
				status: 'failed',
				build_id: buildId,
				phase: input.data.phase,
				error: input.data.error,
				lastLines: input.data.last_lines,
				...(input.data.log_path !== undefined ? { log_path: input.data.log_path } : {})
			};
	}
}
