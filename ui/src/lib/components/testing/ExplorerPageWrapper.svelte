<script lang="ts">
	import ExplorerPage from '../../../routes/explorer/+page.svelte';
	import QueryClientTestWrapper from '$lib/components/testing/QueryClientTestWrapper.svelte';
</script>

<QueryClientTestWrapper>
	<ExplorerPage />
</QueryClientTestWrapper>
