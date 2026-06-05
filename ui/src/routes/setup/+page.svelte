<script lang="ts">
	import { goto } from '$app/navigation';

	function start() {
		goto('/setup/hardware');
	}
</script>

<div class="space-y-6">
	<h1 tabindex="-1" class="text-3xl font-semibold text-[var(--color-text-primary)]">
		Welcome to LlamaForge
	</h1>

	<p class="text-[var(--color-text-secondary)]">
		This wizard walks you through getting LlamaForge running: detecting your hardware, locating (or
		building) <code>llama-server</code>, discovering your models, recommending a configuration, and
		saving it. You can revisit any completed step at any time.
	</p>

	<ol class="list-decimal space-y-1 pl-6 text-sm text-[var(--color-text-secondary)]">
		<li>Detect CPU, RAM, and GPUs</li>
		<li>Find or build the <code>llama-server</code> binary</li>
		<li>Scan local directories for models</li>
		<li>Review a recommended per-model configuration</li>
		<li>Save and launch</li>
	</ol>

	<button
		type="button"
		onclick={start}
		class="rounded bg-[var(--color-focus-ring)] px-6 py-3 text-sm font-medium text-white focus:ring-2 focus:ring-[var(--color-focus-ring)] focus:ring-offset-2 focus:outline-none"
	>
		Start Setup
	</button>
</div>
