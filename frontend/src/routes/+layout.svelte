<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { SystemAlert } from '$lib/types';
	import { onMount } from 'svelte';
	import { AppShell, AppBar } from '@skeletonlabs/skeleton';
	import { page } from '$app/stores';
	import { user, loading, initialize, handleLogout } from '$lib/auth';
	import { themeMode, setThemeMode, initTheme, type ThemeMode } from '$lib/theme';
	import { getSystemAlerts } from '$lib/api';
	import { Permission } from '$lib/types';
	import '../app.postcss';

	let { children }: { children: Snippet } = $props();

	const themeCycle: ThemeMode[] = ['light', 'dark', 'system'];

	let systemAlerts: SystemAlert[] = $state([]);
	let dismissedAlerts: Set<string> = $state(new Set());

	function cycleTheme() {
		const i = themeCycle.indexOf($themeMode);
		setThemeMode(themeCycle[(i + 1) % themeCycle.length]);
	}

	function dismissAlert(id: string) {
		dismissedAlerts = new Set([...dismissedAlerts, id]);
	}

	let visibleAlerts = $derived(systemAlerts.filter((a) => !dismissedAlerts.has(a.id)));

	async function fetchAlerts() {
		try {
			const res = await getSystemAlerts();
			systemAlerts = res.alerts;
		} catch {
			// Silently ignore — alerts are non-critical
		}
	}

	onMount(() => {
		initialize();
		initTheme();
	});

	$effect(() => {
		if ($user?.permissions.includes(Permission.ManageGlobalSettings)) {
			fetchAlerts();
		}
	});

	const publicRoutes = new Set(['/login', '/register', '/device']);

	const allNavItems = [
		{ href: '/', label: 'Home' },
		{ href: '/agents', label: 'Agents' },
		{ href: '/hosts', label: 'Hosts' },
		{ href: '/settings', label: 'Settings', permission: Permission.ViewSettings },
		{ href: '/settings/global', label: 'Global Settings', permission: Permission.ManageGlobalSettings }
	];

	const navItems = $derived(
		allNavItems.filter(
			(item) => !item.permission || $user?.permissions.includes(item.permission)
		)
	);

	let showSidebar = $derived($user && !publicRoutes.has($page.url.pathname));
</script>

{#if $loading}
	<div class="flex h-screen items-center justify-center">
		<p class="text-lg">Loading...</p>
	</div>
{:else}
	<AppShell slotSidebarLeft={showSidebar ? 'w-60 bg-surface-50-900-token border-r border-surface-300-600-token' : 'w-0'}>
		{#snippet header()}
			<AppBar class="border-b border-surface-300-600-token shadow-sm py-1">
				{#snippet lead()}
					<a href="/" class="text-xl font-bold">Uptrakit</a>
				{/snippet}
				{#snippet trail()}
					{#if $user}
						<span class="mr-2">{$user.email}</span>
					{/if}
					<button
						class="btn-icon variant-ghost-surface"
						title={$themeMode === 'light' ? 'Light mode' : $themeMode === 'dark' ? 'Dark mode' : 'System mode'}
						onclick={cycleTheme}
					>
						{#if $themeMode === 'light'}
							<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="w-5 h-5">
								<path d="M10 2a.75.75 0 0 1 .75.75v1.5a.75.75 0 0 1-1.5 0v-1.5A.75.75 0 0 1 10 2Zm0 13a.75.75 0 0 1 .75.75v1.5a.75.75 0 0 1-1.5 0v-1.5A.75.75 0 0 1 10 15Zm-8-5a.75.75 0 0 1 .75-.75h1.5a.75.75 0 0 1 0 1.5h-1.5A.75.75 0 0 1 2 10Zm13 0a.75.75 0 0 1 .75-.75h1.5a.75.75 0 0 1 0 1.5h-1.5A.75.75 0 0 1 15 10Zm-2.05-4.95a.75.75 0 0 1 0 1.06l-1.06 1.06a.75.75 0 0 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0Zm-7.78 7.78a.75.75 0 0 1 0 1.06l-1.06 1.06a.75.75 0 0 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0ZM14.95 12.95a.75.75 0 0 1 0 1.06l-1.06 1.06a.75.75 0 1 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0ZM7.17 5.17a.75.75 0 0 1 0 1.06L6.11 7.29a.75.75 0 0 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0ZM10 6a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z"/>
							</svg>
						{:else if $themeMode === 'dark'}
							<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="w-5 h-5">
								<path fill-rule="evenodd" d="M7.455 2.004a.75.75 0 0 1 .26.77 7 7 0 0 0 9.958 7.967.75.75 0 0 1 1.067.853A8.5 8.5 0 1 1 6.647 1.921a.75.75 0 0 1 .808.083Z" clip-rule="evenodd"/>
							</svg>
						{:else}
							<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="w-5 h-5">
								<path fill-rule="evenodd" d="M2 4.25A2.25 2.25 0 0 1 4.25 2h11.5A2.25 2.25 0 0 1 18 4.25v8.5A2.25 2.25 0 0 1 15.75 15h-3.105a3.501 3.501 0 0 0 1.1 1.677A.75.75 0 0 1 13.26 18H6.74a.75.75 0 0 1-.484-1.323A3.501 3.501 0 0 0 7.355 15H4.25A2.25 2.25 0 0 1 2 12.75v-8.5Zm1.5 0a.75.75 0 0 1 .75-.75h11.5a.75.75 0 0 1 .75.75v7.5a.75.75 0 0 1-.75.75H4.25a.75.75 0 0 1-.75-.75v-7.5Z" clip-rule="evenodd"/>
							</svg>
						{/if}
					</button>
					{#if $user}
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

		{#if visibleAlerts.length > 0}
			<div class="px-4 pt-2 space-y-2">
				{#each visibleAlerts as alert (alert.id)}
					<aside class="alert {alert.severity === 'warning' ? 'variant-filled-warning' : 'variant-filled-surface'}">
						<div class="alert-message">
							<h3 class="h4">{alert.title}</h3>
							<p>{alert.message}</p>
						</div>
						<div class="alert-actions">
							{#if alert.action === 'renew_server_certificate'}
								<a href="/settings/global" class="btn btn-sm variant-filled">Go to Global Settings</a>
							{/if}
							<button class="btn btn-sm variant-soft" onclick={() => dismissAlert(alert.id)}>Dismiss</button>
						</div>
					</aside>
				{/each}
			</div>
		{/if}

		<div class="container mx-auto max-w-2xl p-4">
			{@render children()}
		</div>
	</AppShell>
{/if}
