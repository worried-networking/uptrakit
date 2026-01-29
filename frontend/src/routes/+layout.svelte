<script lang="ts">
	import { onMount } from 'svelte';
	import { AppShell, AppBar } from '@skeletonlabs/skeleton';
	import { user, loading, initialize, handleLogout } from '$lib/auth';
	import '../app.postcss';

	onMount(() => {
		initialize();
	});
</script>

{#if $loading}
	<div class="flex h-screen items-center justify-center">
		<p class="text-lg">Loading...</p>
	</div>
{:else}
	<AppShell>
		{#snippet header()}
			<AppBar>
				{#snippet lead()}
					<a href="/" class="text-xl font-bold">Uptrakit</a>
				{/snippet}
				{#snippet trail()}
					{#if $user}
						<span class="mr-2">{$user.email}</span>
						<button class="btn variant-ghost-surface" onclick={handleLogout}>
							Logout
						</button>
					{:else}
						<a href="/login" class="btn variant-ghost-surface">Login</a>
						<a href="/register" class="btn variant-ghost-surface">Register</a>
					{/if}
				{/snippet}
			</AppBar>
		{/snippet}

		<div class="container mx-auto max-w-2xl p-4">
			<slot />
		</div>
	</AppShell>
{/if}
