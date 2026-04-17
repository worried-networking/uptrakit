<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { SystemAlert } from '$lib/types';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import {
		getUser,
		getLoading,
		initialize,
		handleLogout,
		getSessionExpired,
		setSessionExpired
	} from '$lib/auth.svelte';
	import { getThemeMode, setThemeMode, initTheme, type ThemeMode } from '$lib/theme.svelte';
	import { getSystemAlerts } from '$lib/api';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { Permission, hasPermissionValue } from '$lib/types';
	import {
		loadSurfaceRegistry,
		clearSurfaceRegistry,
		getSurfaceRuntimeStatus,
		getSurfacesBySlot,
		resolveSurfacePageNavItems
	} from '$lib/surfaces/registry.svelte';
	import { Callout } from '$lib/components/ui';
	import ToastNotifications from '$lib/components/ToastNotifications.svelte';
	import '../app.css';

	let { children }: { children: Snippet } = $props();

	const themeCycle: ThemeMode[] = ['light', 'dark', 'system'];

	let systemAlerts: SystemAlert[] = $state([]);
	let dismissedAlerts: Set<string> = $state(new Set());

	function cycleTheme() {
		const i = themeCycle.indexOf(getThemeMode());
		setThemeMode(themeCycle[(i + 1) % themeCycle.length]);
	}

	function dismissAlert(id: string) {
		dismissedAlerts = new Set([...dismissedAlerts, id]);
	}

	type NavItemOrigin = 'built-in' | 'surface.page';
	type ShellNavItem = {
		href: string;
		label: string;
		priority: number;
		origin: NavItemOrigin;
		stableId: string;
	};

	function compareShellNavItems(a: ShellNavItem, b: ShellNavItem): number {
		if (a.priority !== b.priority) return a.priority - b.priority;
		if (a.label !== b.label) return a.label.localeCompare(b.label);
		if (a.origin !== b.origin) return a.origin === 'built-in' ? -1 : 1;
		return a.stableId.localeCompare(b.stableId);
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

	$effect(() => {
		initialize();
		initTheme();
	});

	// Centralized auth guard — redirects unauthenticated users on protected routes
	$effect(() => {
		if (getLoading()) return;

		const path = page.url.pathname;
		const isPublic = publicRoutes.has(path);

		if (!getUser() && !isPublic) {
			goto('/login?redirect=' + encodeURIComponent(path + page.url.search));
		}
	});

	$effect(() => {
		if (getUser()?.permissions.includes(Permission.ManageGlobalSettings)) {
			void fetchAlerts();
		}
	});

	// Load surface registry when authenticated, clear on logout.
	$effect(() => {
		if (getUser()) {
			loadSurfaceRegistry();
		} else {
			clearSurfaceRegistry();
		}
	});

	const publicRoutes = new Set(['/login', '/register', '/device']);

	// Built-in nav items with priority values for unified sorting.
	const builtInNavItems: { href: string; label: string; priority: number; permission?: Permission | Permission[] }[] = [
		{ href: '/', label: 'Home', priority: 100 },
		{ href: '/services', label: 'Services', priority: 200 },
		{ href: '/system-services', label: 'System Services', priority: 300, permission: Permission.ViewSystemServices },
		{ href: '/hosts', label: 'Hosts', priority: 400 },
		{ href: '/host-tags', label: 'Tags', priority: 450, permission: Permission.ViewHosts },
		{ href: '/software', label: 'Software', priority: 500, permission: Permission.ViewSoftware },
		{ href: '/history', label: 'History', priority: 800, permission: Permission.ViewSoftware },
		{ href: '/audit-logs', label: 'Audit Logs', priority: 900, permission: Permission.ViewAuditLogs },
		{
			href: '/settings',
			label: 'Settings',
			priority: 1000,
			permission: [
				Permission.ViewSettings,
				Permission.ManageAuthSettings,
				Permission.ManageEnrollmentTokens,
				Permission.ManageAgentCerts,
				Permission.ViewSoftware,
				Permission.CreateSoftware,
				Permission.UpdateSoftware,
				Permission.DeleteSoftware,
				Permission.ManageScheduler,
				Permission.ManageGlobalSettings
			]
		}
	];

	const surfacePageNavItems = $derived(
		resolveSurfacePageNavItems(
			getSurfacesBySlot('surface.page').filter((surface) => hasPermissionValue(getUser(), surface.required_permission)),
			getSurfaceRuntimeStatus().active
		).map((item) => ({
			id: item.id,
			href: item.href,
			label: item.label,
			priority: item.priority
		}))
	);

	// Merge built-in and surface nav items with deterministic canonical ordering:
	// priority -> label -> origin (built-in first) -> stable ID.
	const navItems = $derived(
		[
			...builtInNavItems
				.filter((item) => {
					if (!item.permission) return true;
					const perms = Array.isArray(item.permission) ? item.permission : [item.permission];
					return perms.some((p) => getUser()?.permissions.includes(p));
				})
				.map(
					(item): ShellNavItem => ({
						href: item.href,
						label: item.label,
						priority: item.priority,
						origin: 'built-in',
						stableId: item.href
					})
				),
			...surfacePageNavItems.map(
				(item): ShellNavItem => ({
					href: item.href,
					label: item.label,
					priority: item.priority,
					origin: 'surface.page',
					stableId: item.id
				})
			)
		].sort(compareShellNavItems)
	);

	function isNavItemActive(item: ShellNavItem): boolean {
		const currentPath = page.url.pathname;
		if (currentPath === item.href) return true;
		if (item.href === '/') return currentPath === '/';
		if (!currentPath.startsWith(item.href + '/')) return false;
		return !navItems.some((other) => other.href !== item.href && currentPath === other.href);
	}

	let showSidebar = $derived(getUser() && !publicRoutes.has(page.url.pathname));
</script>

{#if getLoading()}
	<div class="flex h-screen items-center justify-center">
		<p class="text-lg">Loading...</p>
	</div>
{:else}
	<div class="flex h-full flex-col">
		<!-- Header -->
		<header
			class="relative z-[60] flex items-center justify-between border-b border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-1 shadow-xs"
			data-ui="app-shell-header"
		>
			<a href="#main-content" class="skip-link">Skip to main content</a>
			<a href="/" class="text-xl font-bold">Uptrakit</a>
			<div class="flex items-center gap-2">
				{#if getUser()}
					<a href="/profile" class="mr-2 hover:underline">{getUser()?.email}</a>
				{/if}
				<button
					class="btn-icon preset-tonal-surface"
					title={getThemeMode() === 'light' ? 'Light mode' : getThemeMode() === 'dark' ? 'Dark mode' : 'System mode'}
					onclick={cycleTheme}
				>
					{#if getThemeMode() === 'light'}
						<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="h-5 w-5">
							<path
								d="M10 2a.75.75 0 0 1 .75.75v1.5a.75.75 0 0 1-1.5 0v-1.5A.75.75 0 0 1 10 2Zm0 13a.75.75 0 0 1 .75.75v1.5a.75.75 0 0 1-1.5 0v-1.5A.75.75 0 0 1 10 15Zm-8-5a.75.75 0 0 1 .75-.75h1.5a.75.75 0 0 1 0 1.5h-1.5A.75.75 0 0 1 2 10Zm13 0a.75.75 0 0 1 .75-.75h1.5a.75.75 0 0 1 0 1.5h-1.5A.75.75 0 0 1 15 10Zm-2.05-4.95a.75.75 0 0 1 0 1.06l-1.06 1.06a.75.75 0 0 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0Zm-7.78 7.78a.75.75 0 0 1 0 1.06l-1.06 1.06a.75.75 0 0 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0ZM14.95 12.95a.75.75 0 0 1 0 1.06l-1.06 1.06a.75.75 0 1 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0ZM7.17 5.17a.75.75 0 0 1 0 1.06L6.11 7.29a.75.75 0 0 1-1.06-1.06l1.06-1.06a.75.75 0 0 1 1.06 0ZM10 6a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z"
							/>
						</svg>
					{:else if getThemeMode() === 'dark'}
						<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="h-5 w-5">
							<path
								fill-rule="evenodd"
								d="M7.455 2.004a.75.75 0 0 1 .26.77 7 7 0 0 0 9.958 7.967.75.75 0 0 1 1.067.853A8.5 8.5 0 1 1 6.647 1.921a.75.75 0 0 1 .808.083Z"
								clip-rule="evenodd"
							/>
						</svg>
					{:else}
						<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="h-5 w-5">
							<path
								fill-rule="evenodd"
								d="M2 4.25A2.25 2.25 0 0 1 4.25 2h11.5A2.25 2.25 0 0 1 18 4.25v8.5A2.25 2.25 0 0 1 15.75 15h-3.105a3.501 3.501 0 0 0 1.1 1.677A.75.75 0 0 1 13.26 18H6.74a.75.75 0 0 1-.484-1.323A3.501 3.501 0 0 0 7.355 15H4.25A2.25 2.25 0 0 1 2 12.75v-8.5Zm1.5 0a.75.75 0 0 1 .75-.75h11.5a.75.75 0 0 1 .75.75v7.5a.75.75 0 0 1-.75.75H4.25a.75.75 0 0 1-.75-.75v-7.5Z"
								clip-rule="evenodd"
							/>
						</svg>
					{/if}
				</button>
				{#if getUser()}
					<button class="btn preset-tonal-surface" onclick={handleLogout}> Logout </button>
				{:else}
					<a href="/login" class="btn preset-tonal-surface">Login</a>
					<a href="/register" class="btn preset-tonal-surface">Register</a>
				{/if}
			</div>
		</header>

		{#if !getIsOnline()}
			<div class="px-4 pt-3" data-ui="app-shell-banner">
				<Callout
					tone="warning"
					title="Offline"
					message="You are currently offline. Some features may not be available."
				/>
			</div>
		{/if}

		{#if getSessionExpired()}
			<div class="px-4 pt-3" data-ui="app-shell-banner">
				<Callout tone="danger" title="Session expired" message="Your session has expired.">
					<div class="mt-2 flex flex-wrap items-center gap-2 text-xs">
						<a
							href="/login?redirect={encodeURIComponent(page.url.pathname + page.url.search)}"
							class="btn btn-sm preset-filled-error-500">Log in</a
						>
						<button
							onclick={() => setSessionExpired(false)}
							class="btn btn-sm preset-tonal-surface"
							aria-label="Dismiss session expired notification">Dismiss</button
						>
					</div>
				</Callout>
			</div>
		{/if}

		<!-- Body -->
		<div class="flex min-h-0 flex-1">
			<!-- Sidebar -->
			{#if showSidebar}
				<aside
					class="relative z-[60] w-60 border-r border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4"
					data-ui="app-shell-sidebar"
				>
					<nav data-ui="app-shell-nav">
						<ul class="space-y-1">
							{#each navItems as item (item.href)}
								<li>
									<a
										href={item.href}
										class={`block rounded-md px-3 py-2 text-sm font-medium transition-colors ${
											isNavItemActive(item)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										}`}
										data-ui="app-shell-nav-item"
									>
										{item.label}
									</a>
								</li>
							{/each}
						</ul>
					</nav>
				</aside>
			{/if}

			<!-- Main content -->
			<main id="main-content" class="flex-1 overflow-auto">
				<ToastNotifications alerts={visibleAlerts} onDismiss={dismissAlert} />

				<div class="container mx-auto max-w-5xl p-4">
					{#if getUser() || publicRoutes.has(page.url.pathname)}
						{@render children()}
					{/if}
				</div>
			</main>
		</div>
	</div>
{/if}
