<script lang="ts">
	import DynamicHeading from '../shared/DynamicHeading.svelte';

	let { level = 1 }: { level: 1 | 2 | 3 | 4 | 5 | 6 } = $props();
</script>

<DynamicHeading {level}>
	<span>Heading Content</span>
</DynamicHeading>
