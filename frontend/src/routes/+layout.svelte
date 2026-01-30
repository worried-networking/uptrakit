<script lang="ts">
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import { AppShell, AppBar } from '@skeletonlabs/skeleton';
	import { page } from '$app/stores';
	import { user, loading, initialize, handleLogout } from '$lib/auth';
	import '../app.postcss';

	let { children }: { children: Snippet } = $props();

	onMount(() => {
		initialize();
	});

	const publicRoutes = new Set(['/login', '/register']);

	const navItems = [
		{ href: '/', label: 'Home' },
		{ href: '/agents', label: 'Agents' },
		{ href: '/settings', label: 'Settings' }
	];

	let showSidebar = $derived($user && !publicRoutes.has($page.url.pathname));
</script>

{#if $loading}
	<div class="flex h-screen items-center justify-center">
		<p class="text-lg">Loading...</p>
	</div>
{:else}
	<AppShell slotSidebarLeft={showSidebar ? 'w-60 bg-surface-50-900-token border-r border-surface-300-600-token' : 'w-0'}>
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

		{#snippet sidebarLeft()}
			{#if showSidebar}
				<nav class="list-nav p-4">
					<ul>
						{#each navItems as item}
							<li>
								<a
									href={item.href}
									class={$page.url.pathname === item.href ? 'bg-primary-active-token' : ''}
								>
									{item.label}
								</a>
							</li>
						{/each}
					</ul>
				</nav>
			{/if}
		{/snippet}

		<div class="container mx-auto max-w-2xl p-4">
			{@render children()}
		</div>
	</AppShell>
{/if}
