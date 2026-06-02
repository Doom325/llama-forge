<script lang="ts">
	import GlobalSettingsEditor from '$lib/components/config/GlobalSettingsEditor.svelte';
	import QueryClientTestWrapper from '$lib/components/testing/QueryClientTestWrapper.svelte';
	import type { GlobalSettings } from '$lib/api/types';

	let {
		initial,
		etag = 'v1',
		reload
	}: {
		initial: GlobalSettings;
		etag?: string | null;
		reload: () => Promise<{ data: GlobalSettings; etag: string | null }>;
	} = $props();
</script>

<QueryClientTestWrapper>
	<GlobalSettingsEditor {initial} {etag} {reload} />
</QueryClientTestWrapper>
