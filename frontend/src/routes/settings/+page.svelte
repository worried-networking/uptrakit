<script lang="ts">
	import { user } from '$lib/auth';
	import { goto } from '$app/navigation';

	$effect(() => {
		if (!$user) {
			goto('/login');
		}
	});
</script>

{#if $user}
	<h1 class="h1 mb-6">Settings</h1>

	<div class="card p-8 text-center">
		<p>Settings page coming soon.</p>
	</div>
{/if}
