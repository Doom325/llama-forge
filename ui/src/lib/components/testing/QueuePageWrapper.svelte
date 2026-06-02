<script lang="ts">
	import QueuePage from '../../../routes/queue/+page.svelte';
	import QueryClientTestWrapper from '$lib/components/testing/QueryClientTestWrapper.svelte';
</script>

<QueryClientTestWrapper>
	<QueuePage />
</QueryClientTestWrapper>
