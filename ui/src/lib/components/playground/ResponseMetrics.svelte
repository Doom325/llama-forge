<script lang="ts">
	import type { ResponseMetrics } from '$lib/api/types';

	let { metrics }: { metrics: ResponseMetrics } = $props();
</script>

<div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-[var(--color-text-secondary)]">
	<span>TTFT: {Math.round(metrics.ttft)} ms</span>
	<span>{metrics.tokensPerSecond.toFixed(1)} tok/s</span>
	<span>{metrics.totalTokens} tokens</span>
</div>
