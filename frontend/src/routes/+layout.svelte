<script lang="ts">
	import { onMount } from 'svelte';
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
		getSurfacesBySlot,
		resolveSurfacePageNavItems
	} from '$lib/surfaces/registry.svelte';
	import { Callout } from '$lib/components/ui';
	import ToastNotifications from '$lib/components/ToastNotifications.svelte';
	import Button from '$lib/components/Button.svelte';
	import '../app.css';

	let { children }: { children: Snippet } = $props();

	const TABLET_BREAKPOINT = 640;
	const DESKTOP_BREAKPOINT = 1024;
	const FOCUSABLE =
		'button:not([disabled]), [href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
	const themeCycle: ThemeMode[] = ['light', 'dark', 'system'];

	let systemAlerts: SystemAlert[] = $state([]);
	let dismissedAlerts: Set<string> = $state(new Set());
	let viewportWidth = $state<number>(DESKTOP_BREAKPOINT);
	let sidebarOverlayOpen = $state(false);
	let mobileOverflowOpen = $state(false);
	let shellHeaderEl: HTMLElement | undefined = $state(undefined);
	let shellBannerRegionEl: HTMLDivElement | undefined = $state(undefined);
	let shellMainEl: HTMLElement | undefined = $state(undefined);
	let tabletSidebarEl: HTMLElement | undefined = $state(undefined);
	let mobileOverflowSheetEl: HTMLDivElement | undefined = $state(undefined);
	let mobileOverflowToggleEl: HTMLButtonElement | undefined = $state(undefined);
	let mobileNavEl: HTMLElement | undefined = $state(undefined);

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
	let currentPageStatus = $derived(typeof page.status === 'number' ? page.status : 200);

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

	onMount(() => {
		const syncViewport = () => {
			viewportWidth = window.innerWidth;
		};

		syncViewport();
		window.addEventListener('resize', syncViewport);

		return () => {
			window.removeEventListener('resize', syncViewport);
		};
	});

	// Centralized auth guard — redirects unauthenticated users on protected routes
	$effect(() => {
		if (getLoading()) return;

		const path = page.url.pathname;
		const isPublic = publicRoutes.has(path);
		const isPublicError = currentPageStatus >= 400;

		if (!getUser() && !isPublic && !isPublicError) {
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
			getSurfacesBySlot('surface.page').filter((surface) => hasPermissionValue(getUser(), surface.required_permission))
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
	const isTablet = $derived(viewportWidth >= TABLET_BREAKPOINT && viewportWidth < DESKTOP_BREAKPOINT);
	const isMobile = $derived(viewportWidth < TABLET_BREAKPOINT);
	const mobilePrimaryNavItems = $derived(navItems.slice(0, 4));
	const mobileOverflowNavItems = $derived(navItems.slice(4));
	const mobileOverflowActive = $derived(mobileOverflowNavItems.some((item) => isNavItemActive(item)));

	function isNavItemActive(item: ShellNavItem): boolean {
		const currentPath = page.url.pathname;
		if (currentPath === item.href) return true;
		if (item.href === '/') return currentPath === '/';
		if (!currentPath.startsWith(item.href + '/')) return false;
		return !navItems.some((other) => other.href !== item.href && currentPath === other.href);
	}

	function closeTransientNavigation() {
		sidebarOverlayOpen = false;
		mobileOverflowOpen = false;
	}

	function getFocusableElements(root: HTMLElement): HTMLElement[] {
		return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
			(element) => !element.hasAttribute('disabled') && element.getAttribute('aria-hidden') !== 'true'
		);
	}

	function activateOverlayModal(
		root: HTMLElement | undefined,
		inertTargets: Array<HTMLElement | undefined>,
		onclose: () => void,
		restoreFocusTo?: HTMLElement
	) {
		if (!root) return;
		const overlayRoot = root;

		const previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		const activeTargets = inertTargets.filter((target): target is HTMLElement => target !== undefined);

		for (const target of activeTargets) {
			target.inert = true;
			target.setAttribute('aria-hidden', 'true');
		}

		queueMicrotask(() => {
			const [firstFocusable] = getFocusableElements(overlayRoot);
			(firstFocusable ?? overlayRoot).focus();
		});

		function handleKeydown(event: KeyboardEvent) {
			if (event.key === 'Escape') {
				event.preventDefault();
				onclose();
				return;
			}

			if (event.key !== 'Tab') return;

			const focusable = getFocusableElements(overlayRoot);
			if (focusable.length === 0) {
				event.preventDefault();
				overlayRoot.focus();
				return;
			}

			const first = focusable[0];
			const last = focusable[focusable.length - 1];

			if (event.shiftKey && document.activeElement === first) {
				event.preventDefault();
				last.focus();
			} else if (!event.shiftKey && document.activeElement === last) {
				event.preventDefault();
				first.focus();
			}
		}

		document.addEventListener('keydown', handleKeydown, true);

		return () => {
			document.removeEventListener('keydown', handleKeydown, true);
			for (const target of activeTargets) {
				target.inert = false;
				target.removeAttribute('aria-hidden');
			}

			const focusTarget = restoreFocusTo ?? previouslyFocused;
			queueMicrotask(() => {
				focusTarget?.focus();
			});
		};
	}

	let showShellChrome = $derived(getUser() && !publicRoutes.has(page.url.pathname) && currentPageStatus < 400);

	$effect(() => {
		void page.url.pathname;
		closeTransientNavigation();
	});

	$effect(() => {
		if (!showShellChrome) {
			closeTransientNavigation();
			return;
		}
		if (!isTablet) {
			sidebarOverlayOpen = false;
		}
		if (!isMobile) {
			mobileOverflowOpen = false;
		}
	});

	$effect(() => {
		if (!sidebarOverlayOpen && !mobileOverflowOpen) return;

		const previousOverflow = document.body.style.overflow;
		document.body.style.overflow = 'hidden';

		return () => {
			document.body.style.overflow = previousOverflow;
		};
	});

	$effect(() => {
		if (!sidebarOverlayOpen) return;

		const toggleEl =
			(document.querySelector('[data-ui="app-shell-sidebar-toggle"]') as HTMLElement | null) ?? undefined;

		return activateOverlayModal(
			tabletSidebarEl,
			[shellHeaderEl, shellBannerRegionEl, shellMainEl],
			() => {
				sidebarOverlayOpen = false;
			},
			toggleEl
		);
	});

	$effect(() => {
		if (!mobileOverflowOpen) return;

		return activateOverlayModal(
			mobileOverflowSheetEl,
			[shellHeaderEl, shellBannerRegionEl, shellMainEl, mobileNavEl],
			() => {
				mobileOverflowOpen = false;
			},
			mobileOverflowToggleEl
		);
	});
</script>

{#if getLoading()}
	<div class="flex h-screen items-center justify-center">
		<p class="text-lg">Loading...</p>
	</div>
{:else}
	<div class="flex h-full flex-col">
		<!-- Header -->
		<header
			bind:this={shellHeaderEl}
			class="relative flex h-10 items-center justify-between border-b border-[var(--border-subtle)] bg-[var(--bg-surface)] content-padding-x shadow-xs"
			data-ui="app-shell-header"
		>
			<a href="#main-content" class="skip-link">Skip to main content</a>
			<div class="flex min-w-0 items-center gap-2">
				{#if showShellChrome && isTablet}
					<Button
						variant="ghost"
						ariaLabel={sidebarOverlayOpen ? 'Close navigation' : 'Open navigation'}
						aria-controls="app-shell-sidebar-tablet"
						aria-expanded={sidebarOverlayOpen}
						data-ui="app-shell-sidebar-toggle"
						onclick={() => (sidebarOverlayOpen = !sidebarOverlayOpen)}
					>
						{#snippet leadingIcon()}
							<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" class="h-4 w-4">
								<path
									fill-rule="evenodd"
									d="M2 4.75A.75.75 0 0 1 2.75 4h14.5a.75.75 0 0 1 0 1.5H2.75A.75.75 0 0 1 2 4.75Zm0 5A.75.75 0 0 1 2.75 9h14.5a.75.75 0 0 1 0 1.5H2.75A.75.75 0 0 1 2 9.75Zm0 5A.75.75 0 0 1 2.75 14h14.5a.75.75 0 0 1 0 1.5H2.75A.75.75 0 0 1 2 14.75Z"
									clip-rule="evenodd"
								/>
							</svg>
						{/snippet}
					</Button>
				{/if}
				<a href="/" class="truncate text-table-body font-bold tracking-nav text-[var(--text-primary)]">Uptrakit</a>
			</div>
			<div class="flex items-center gap-1.5">
				{#if getUser()}
					<a href="/profile" class="hidden text-sm text-[var(--text-secondary)] hover:underline sm:inline">
						{getUser()?.email}
					</a>
				{/if}
				<Button
					variant="ghost"
					ariaLabel={getThemeMode() === 'light'
						? 'Light mode — click to switch to dark'
						: getThemeMode() === 'dark'
							? 'Dark mode — click to switch to system'
							: 'System mode — click to switch to light'}
					onclick={cycleTheme}
				>
					{#snippet leadingIcon()}
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
					{/snippet}
				</Button>
				{#if getUser()}
					<Button variant="danger" onclick={handleLogout}>Logout</Button>
				{:else}
					<Button variant="ghost" href="/login">Login</Button>
					<Button variant="ghost" href="/register">Register</Button>
				{/if}
			</div>
		</header>

		<div bind:this={shellBannerRegionEl}>
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
							<Button
								variant="danger"
								size="sm"
								href="/login?redirect={encodeURIComponent(page.url.pathname + page.url.search)}">Log in</Button
							>
							<Button variant="ghost" size="sm" onclick={() => setSessionExpired(false)}>Dismiss</Button>
						</div>
					</Callout>
				</div>
			{/if}
		</div>

		<!-- Body -->
		<div class="flex min-h-0 flex-1">
			{#if showShellChrome && !isTablet && !isMobile}
				<aside
					class="relative w-sidebar shrink-0 border-r border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-3"
					data-ui="app-shell-sidebar"
					data-variant="desktop"
				>
					<nav data-ui="app-shell-nav">
						<ul class="space-y-0.5">
							{#each navItems as item (item.href)}
								<li>
									<a
										href={item.href}
										class={`flex h-7 items-center rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										}`}
										aria-current={isNavItemActive(item) ? 'page' : undefined}
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
			{#if showShellChrome && isTablet}
				{#if sidebarOverlayOpen}
					<button
						class="fixed inset-x-0 bottom-0 top-10 bg-black/40"
						type="button"
						aria-label="Close navigation"
						data-ui="app-shell-sidebar-backdrop"
						onclick={() => (sidebarOverlayOpen = false)}
					></button>
				{/if}
				<aside
					bind:this={tabletSidebarEl}
					id="app-shell-sidebar-tablet"
					class={`fixed bottom-0 left-0 top-10 w-sidebar border-r border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-3 transition-[transform,opacity] duration-200 ease-out ${
						sidebarOverlayOpen ? 'translate-x-0 opacity-100' : 'pointer-events-none -translate-x-sidebar opacity-0'
					}`}
					class:invisible={!sidebarOverlayOpen}
					aria-hidden={!sidebarOverlayOpen}
					tabindex="-1"
					data-ui="app-shell-sidebar"
					data-variant="tablet"
				>
					<nav data-ui="app-shell-nav">
						<ul class="space-y-0.5">
							{#each navItems as item (item.href)}
								<li>
									<a
										href={item.href}
										class={`flex h-7 items-center rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										}`}
										aria-current={isNavItemActive(item) ? 'page' : undefined}
										data-ui="app-shell-nav-item"
										onclick={() => (sidebarOverlayOpen = false)}
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
			<main bind:this={shellMainEl} id="main-content" class="flex-1 overflow-auto">
				<ToastNotifications alerts={visibleAlerts} onDismiss={dismissAlert} />

				<div
					class={`mx-auto max-w-5xl content-padding-x py-3 ${showShellChrome && isMobile ? 'pb-[calc(12px+4.5rem+env(safe-area-inset-bottom))]' : ''}`}
				>
					{#if getUser() || publicRoutes.has(page.url.pathname) || currentPageStatus >= 400}
						{@render children()}
					{/if}
				</div>
			</main>
		</div>
		{#if showShellChrome && isMobile}
			<nav
				bind:this={mobileNavEl}
				class="fixed inset-x-0 bottom-0 border-t border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2 pt-2 pb-[calc(0.5rem+env(safe-area-inset-bottom))]"
				data-ui="app-shell-mobile-nav"
			>
				<div class="mx-auto flex max-w-5xl items-stretch gap-1">
					{#each mobilePrimaryNavItems as item (item.href)}
						<a
							href={item.href}
							class={`flex min-w-0 flex-1 items-center justify-center rounded-card px-1 py-1.5 text-center text-nav-item font-medium leading-tight transition-colors ${
								isNavItemActive(item)
									? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
									: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
							}`}
							aria-current={isNavItemActive(item) ? 'page' : undefined}
							data-ui="app-shell-mobile-nav-item"
							onclick={closeTransientNavigation}
						>
							<span class="truncate">{item.label}</span>
						</a>
					{/each}
					{#if mobileOverflowNavItems.length > 0}
						<button
							bind:this={mobileOverflowToggleEl}
							class={`flex min-w-0 flex-1 items-center justify-center rounded-card px-1 py-1.5 text-center text-nav-item font-medium leading-tight transition-colors ${
								mobileOverflowOpen || mobileOverflowActive
									? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
									: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
							}`}
							type="button"
							aria-expanded={mobileOverflowOpen}
							aria-controls="app-shell-mobile-overflow-sheet"
							data-ui="app-shell-mobile-nav-item"
							onclick={() => (mobileOverflowOpen = !mobileOverflowOpen)}
						>
							<span class="truncate">More</span>
						</button>
					{/if}
				</div>
			</nav>
			{#if mobileOverflowOpen}
				<button
					class="fixed inset-0 bg-black/40"
					type="button"
					aria-label="Close more navigation"
					data-ui="app-shell-mobile-overflow-backdrop"
					onclick={() => (mobileOverflowOpen = false)}
				></button>
				<div
					bind:this={mobileOverflowSheetEl}
					id="app-shell-mobile-overflow-sheet"
					class="fixed inset-x-0 bottom-0 rounded-t-panel border-t border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 pt-3 pb-[calc(1rem+env(safe-area-inset-bottom))] shadow-xl"
					tabindex="-1"
					data-ui="app-shell-mobile-overflow-sheet"
				>
					<nav data-ui="app-shell-nav">
						<ul class="space-y-0.5">
							{#each mobileOverflowNavItems as item (item.href)}
								<li>
									<a
										href={item.href}
										class={`flex h-7 items-center rounded-card px-2.5 text-nav-item font-medium tracking-nav transition-colors ${
											isNavItemActive(item)
												? 'bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]'
												: 'text-[var(--text-secondary)] hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]'
										}`}
										aria-current={isNavItemActive(item) ? 'page' : undefined}
										data-ui="app-shell-nav-item"
										onclick={() => (mobileOverflowOpen = false)}
									>
										{item.label}
									</a>
								</li>
							{/each}
						</ul>
					</nav>
				</div>
			{/if}
		{/if}
	</div>
{/if}
