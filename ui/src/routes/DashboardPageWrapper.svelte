<script lang="ts">
	import DashboardPage from './+page.svelte';
	import QueryClientTestWrapper from '$lib/components/testing/QueryClientTestWrapper.svelte';
</script>

<QueryClientTestWrapper>
	<DashboardPage />
</QueryClientTestWrapper>
