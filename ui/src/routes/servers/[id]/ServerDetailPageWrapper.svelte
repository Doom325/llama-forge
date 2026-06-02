<script lang="ts">
	import ServerDetailPage from './+page.svelte';
	import QueryClientTestWrapper from '$lib/components/testing/QueryClientTestWrapper.svelte';
</script>

<QueryClientTestWrapper>
	<ServerDetailPage />
</QueryClientTestWrapper>
