// Shared helper to build a server config draft by merging a model + its
// recommendation + the user's override. Used by both the Configure step (per
// card) and the Review step (to assemble the SaveConfigRequest), so the merge
// rule lives in exactly one place.

import type { ConfigRecommendation, DiscoveredModel, ServerConfigDraft } from '$lib/api/types';

/** Default alias for a model file (filename without the `.gguf` extension). */
export function defaultAlias(filename: string): string {
	return filename.replace(/\.gguf$/i, '');
}

/**
 * The effective config draft for a model: the recommendation as the base, the
 * user's override applied on top. The override always wins (matches the
 * configure-step guard's precedence).
 */
export function mergeServerDraft(
	model: DiscoveredModel,
	recommendation: ConfigRecommendation,
	binaryPath: string,
	override: Partial<ServerConfigDraft>
): ServerConfigDraft {
	return {
		model_path: model.path,
		alias: defaultAlias(model.filename),
		binary_path: binaryPath,
		port: recommendation.port,
		gpu_layers: recommendation.gpu_layers,
		parallel_slots: recommendation.parallel_slots,
		ctx_size: recommendation.ctx_size,
		cache_type_k: recommendation.cache_type_k,
		cache_type_v: recommendation.cache_type_v,
		flash_attn: recommendation.flash_attn,
		estimated_vram_mb: recommendation.estimated_vram_mb,
		...override
	};
}
