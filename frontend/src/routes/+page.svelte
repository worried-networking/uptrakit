<script lang="ts">
	import { user } from '$lib/auth';
</script>

{#if $user}
	<div class="card p-8 text-center">
		<h1 class="h1">Hello {$user.first_name} {$user.last_name}</h1>
	</div>
{/if}
