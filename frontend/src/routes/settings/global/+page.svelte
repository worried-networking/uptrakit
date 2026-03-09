<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';

	onMount(() => {
		goto('/settings?tab=global-settings', { replaceState: true });
	});
</script>
