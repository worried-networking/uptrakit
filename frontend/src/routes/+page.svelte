<script lang="ts">
	import { getUser } from '$lib/auth.svelte';
</script>

{#if getUser()}
	<div class="card p-8 text-center">
		<h1 class="h1">Hello {getUser()?.first_name} {getUser()?.last_name}</h1>
	</div>
{/if}
