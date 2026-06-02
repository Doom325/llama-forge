<script lang="ts">
	import ServerDetailPage from '../../../routes/servers/[id]/+page.svelte';
	import QueryClientTestWrapper from '$lib/components/testing/QueryClientTestWrapper.svelte';
</script>

<QueryClientTestWrapper>
	<ServerDetailPage />
</QueryClientTestWrapper>
