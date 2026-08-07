<script lang="ts">
	import { getUser } from '$lib/auth.svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getCombinedSettings, listProviders } from '$lib/api';
	import type { AgentCertificateSettingsResponse } from '$lib/api';
	import type { EnrollmentTokensSummary } from '$lib/api';
	import type { OidcProviderResponse } from '$lib/api';
	import { Actions, hasAction, hasAnyAction, hasActionValue, isAuthorityUnavailable } from '$lib/api';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import {
		getSurfaceReadLoading,
		getSurfaceReadModel,
		getSurfaceReadRequested,
		getSurfaceRegistryLoaded,
		getSurfacesBySlot,
		loadSurfaceReadModels
	} from '$lib/surfaces/registry.svelte';
	import { filterSurfacesByAction, isSurfaceTabPending } from '$lib/surfaces/read-model';
	import type { SurfaceResponse } from '$lib/surfaces/contract';
	import { SvelteMap } from 'svelte/reactivity';
	import { Callout, PageShell, SectionCard, TabStrip, type TabStripItem } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	import AccessSettings from './AccessSettings.svelte';
	import McpAccessTab from './McpAccessTab.svelte';
	import OidcProvidersSettings from './OidcProvidersSettings.svelte';
	import AgentCertificateSettings from './AgentCertificateSettings.svelte';
	import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';
	import PluginConfigsTab from './PluginConfigsTab.svelte';
	import SchedulerTab from './SchedulerTab.svelte';
	import GlobalSettingsTab from './GlobalSettingsTab.svelte';
	import NotificationRulesSettings from './NotificationRulesSettings.svelte';
	import NotificationLogView from './NotificationLogView.svelte';
	import DangerZone from './DangerZone.svelte';
	import InstanceConfigTab from './InstanceConfigTab.svelte';

	// Maintenance: keep in sync with the {#if activeTab === '...'} chain below.
	// Surfaces with a matching tab_group append to these existing tabs instead of creating new ones.
	const BUILTIN_TAB_IDS = new Set([
		'general',
		'mcp-access',
		'plugin-configs',
		'scheduler',
		'global-settings',
		'notification-rules',
		'notification-log',
		'instance-config'
	]);

	// ── Actions ─────────────────────────────────────────────────────────
	const canManageSettings = $derived(
		hasAnyAction(
			getUser(),
			Actions.SETTINGS_AUTH_MANAGE,
			Actions.SETTINGS_ENROLLMENT_TOKENS_MANAGE,
			Actions.SETTINGS_CERTIFICATES_MANAGE
		)
	);
	const canManageOAuthClients = $derived(hasAction(getUser(), Actions.SETTINGS_AUTH_MANAGE));
	const canViewSoftware = $derived(hasAction(getUser(), Actions.SOFTWARE_READ));
	const canViewTypeSettings = $derived(hasAnyAction(getUser(), Actions.SETTINGS_READ, Actions.SYSTEM_SETTINGS_MANAGE));
	const canManageSoftware = $derived(
		hasAnyAction(
			getUser(),
			Actions.SOFTWARE_CREATE,
			Actions.SOFTWARE_UPDATE,
			Actions.SOFTWARE_DELETE,
			Actions.CHECKS_TRIGGER,
			Actions.UPDATES_TRIGGER,
			Actions.SCHEDULER_MANAGE
		)
	);
	const canManageGlobalSettings = $derived(hasAction(getUser(), Actions.SYSTEM_SETTINGS_MANAGE));
	const canViewNotifications = $derived(hasAction(getUser(), Actions.NOTIFICATIONS_READ));
	const canViewInstanceConfig = $derived(hasAction(getUser(), Actions.SYSTEM_CONFIG_STATE_READ));
	const hasAnyTabAction = $derived(
		canManageSettings ||
			canManageOAuthClients ||
			canViewSoftware ||
			canViewTypeSettings ||
			canManageSoftware ||
			canManageGlobalSettings ||
			canViewNotifications ||
			canViewInstanceConfig
	);

	// ── Tab state ────────────────────────────────────────────────────────
	const slotTabSurfaces = $derived(
		filterSurfacesByAction(getSurfacesBySlot('settings.tabs'), (requiredAction) =>
			hasActionValue(getUser(), requiredAction)
		)
	);
	const slotTabReads = $derived.by(() => {
		const result: Record<string, NonNullable<ReturnType<typeof getSurfaceReadModel>>> = {};
		for (const surface of slotTabSurfaces) {
			const read = getSurfaceReadModel(surface.surface_id);
			if (read) {
				result[surface.surface_id] = read;
			}
		}
		return result;
	});
	type TabGroup = { id: string; label: string; surfaces: SurfaceResponse[] };
	const slotTabGroups = $derived.by<TabGroup[]>(() => {
		const groups = new SvelteMap<string, TabGroup>();
		for (const surface of slotTabSurfaces) {
			const key = surface.tab_group ?? surface.surface_id;
			const existing = groups.get(key);
			if (existing) {
				existing.surfaces.push(surface);
			} else {
				groups.set(key, {
					id: key,
					label: surface.tab_group_label ?? surface.label,
					surfaces: [surface]
				});
			}
		}
		return [...groups.values()];
	});
	const slotTabGroupsByTabId = $derived(new SvelteMap(slotTabGroups.map((g) => [g.id, g])));
	const showSurfaceSettingsTabs = $derived(slotTabGroups.some((g) => !BUILTIN_TAB_IDS.has(g.id)));
	const tabItems = $derived.by<TabStripItem[]>(() => {
		const items: TabStripItem[] = [];
		if (canManageSettings) {
			items.push({ id: 'general', label: 'General' });
		}
		if (canManageOAuthClients) {
			items.push({ id: 'mcp-access', label: 'MCP Access' });
		}
		if (canViewSoftware || canViewTypeSettings) {
			items.push({ id: 'plugin-configs', label: 'Plugin Configs' });
		}
		if (canManageSoftware) {
			items.push({ id: 'scheduler', label: 'Scheduler' });
		}
		if (canManageGlobalSettings) {
			items.push({ id: 'global-settings', label: 'Global Settings' });
		}
		if (canViewNotifications) {
			items.push({ id: 'notification-rules', label: 'Notification Rules' });
			items.push({ id: 'notification-log', label: 'Notification Log' });
		}
		if (canViewInstanceConfig) {
			items.push({ id: 'instance-config', label: 'Instance Configuration' });
		}
		if (showSurfaceSettingsTabs) {
			for (const group of slotTabGroups) {
				if (!BUILTIN_TAB_IDS.has(group.id)) {
					items.push({ id: group.id, label: group.label });
				}
			}
		}
		return items;
	});

	let activeTab: string = $state(page.url.searchParams.get('tab') ?? 'general');
	const builtinAppendGroup = $derived(BUILTIN_TAB_IDS.has(activeTab) ? slotTabGroupsByTabId.get(activeTab) : undefined);

	// Redirect if no permissions at all. Skipped when authority is unavailable: the
	// empty `actions` array in that state is a degraded-read placeholder, not a genuine
	// denial, so bouncing the user off the page would defeat the fail-open contract.
	$effect(() => {
		const user = getUser();
		if (user && !hasAnyTabAction && !isAuthorityUnavailable(user)) {
			goto('/');
		}
	});

	// Correct activeTab if it's not accessible
	$effect(() => {
		const user = getUser();
		if (!user) return;
		const surfaceRegistryLoaded = getSurfaceRegistryLoaded();
		const isBuiltinAccessible =
			(activeTab === 'general' && canManageSettings) ||
			(activeTab === 'mcp-access' && canManageOAuthClients) ||
			(activeTab === 'plugin-configs' && (canViewSoftware || canViewTypeSettings)) ||
			(activeTab === 'scheduler' && canManageSoftware) ||
			(activeTab === 'global-settings' && canManageGlobalSettings) ||
			(activeTab === 'notification-rules' && canViewNotifications) ||
			(activeTab === 'notification-log' && canViewNotifications) ||
			(activeTab === 'instance-config' && canViewInstanceConfig);
		const isSurfaceAccessible =
			showSurfaceSettingsTabs && slotTabGroups.some((g) => g.id === activeTab && !BUILTIN_TAB_IDS.has(g.id));
		const activeGroup = !BUILTIN_TAB_IDS.has(activeTab) ? slotTabGroupsByTabId.get(activeTab) : undefined;
		const isPendingSurfaceTab = activeGroup
			? activeGroup.surfaces.some((surface) =>
					isSurfaceTabPending({
						activeTab: surface.surface_id,
						slotSurfaces: [surface],
						readBySurface: slotTabReads,
						isReadRequested: getSurfaceReadRequested(surface.surface_id),
						isReadLoading: getSurfaceReadLoading(surface.surface_id)
					})
				)
			: false;
		if (!surfaceRegistryLoaded && !isBuiltinAccessible) {
			return;
		}
		if (!isBuiltinAccessible && !isSurfaceAccessible && !isPendingSurfaceTab) {
			if (canManageSettings) activeTab = 'general';
			else if (canManageOAuthClients) activeTab = 'mcp-access';
			else if (canViewSoftware || canViewTypeSettings) activeTab = 'plugin-configs';
			else if (canManageSoftware) activeTab = 'scheduler';
			else if (canManageGlobalSettings) activeTab = 'global-settings';
			else if (canViewNotifications) activeTab = 'notification-rules';
			else if (canViewInstanceConfig) activeTab = 'instance-config';
			else if (showSurfaceSettingsTabs) {
				const firstNewGroup = slotTabGroups.find((g) => !BUILTIN_TAB_IDS.has(g.id));
				if (firstNewGroup) activeTab = firstNewGroup.id;
			}
		}
	});

	// Sync active tab to URL
	$effect(() => {
		const search = activeTab !== 'general' ? `?tab=${activeTab}` : '';
		goto(search ? `${location.pathname}${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	// ── General tab state ─────────────────────────────────────────────────
	let loading: boolean = $state(true);

	let oidcProviders: OidcProviderResponse[] | undefined = $state(undefined);
	let agentCertSettings: AgentCertificateSettingsResponse | undefined = $state(undefined);
	let enrollmentTokensSummary: EnrollmentTokensSummary | undefined = $state(undefined);
	let multiTenancyEnabled: boolean = $state(false);

	let oidcProvidersError: string | null = $state(null);
	let agentCertificateError: string | null = $state(null);
	let enrollmentTokenError: string | null = $state(null);

	$effect(() => {
		if (canManageSettings) {
			loadAllSettings();
		}
	});

	$effect(() => {
		if (slotTabSurfaces.length === 0) {
			return;
		}
		void loadSurfaceReadModels(slotTabSurfaces.map((surface) => surface.surface_id));
	});

	async function loadAllSettings() {
		loading = true;
		oidcProvidersError = null;
		agentCertificateError = null;
		enrollmentTokenError = null;

		const results = await Promise.allSettled([getCombinedSettings(), listProviders()]);

		if (results[0].status === 'fulfilled') {
			const combined = results[0].value.data;
			agentCertSettings = combined.agent_certificates;
			enrollmentTokensSummary = combined.enrollment_tokens;
			multiTenancyEnabled = combined.multi_tenancy_enabled;
		} else {
			const msg = results[0].reason instanceof Error ? results[0].reason.message : 'Failed to load combined settings.';
			agentCertificateError = msg;
			enrollmentTokenError = msg;
		}

		if (results[1].status === 'fulfilled') {
			oidcProviders = results[1].value.data;
		} else {
			oidcProvidersError =
				results[1].reason instanceof Error ? results[1].reason.message : 'Failed to load OIDC providers.';
		}

		loading = false;
	}
</script>

{#if getUser() && hasAnyTabAction}
	<PageShell title="Settings" description="Configure authentication, plugins, scheduling, and global behavior.">
		<TabStrip
			items={tabItems}
			activeId={activeTab}
			ariaLabel="Settings tabs"
			idBase="settings"
			onSelect={(id) => (activeTab = id)}
		/>

		<!-- General tab -->
		{#if activeTab === 'general'}
			{#if loading}
				<SectionCard title="General Settings">
					<p class="py-4 text-center text-sm text-[var(--text-secondary)]">Loading settings...</p>
				</SectionCard>
			{/if}

			<div aria-busy={loading} class="space-y-4" class:opacity-50={loading}>
				<AccessSettings onSuccess={showSuccess} onError={showError} />
				<OidcProvidersSettings
					providers={oidcProviders}
					{multiTenancyEnabled}
					onSuccess={showSuccess}
					onError={showError}
				/>
				{#if oidcProvidersError}
					<Callout tone="danger" title="Unable to load OIDC providers" message={oidcProvidersError}>
						<div class="mt-2">
							<Button variant="primary" size="sm" onclick={() => loadAllSettings()}>Retry All</Button>
						</div>
					</Callout>
				{/if}
				<AgentCertificateSettings settings={agentCertSettings} onSuccess={showSuccess} onError={showError} />
				{#if agentCertificateError}
					<Callout tone="danger" title="Unable to load certificate settings" message={agentCertificateError}>
						<div class="mt-2">
							<Button variant="primary" size="sm" onclick={() => loadAllSettings()}>Retry All</Button>
						</div>
					</Callout>
				{/if}
				<EnrollmentTokenSettings summary={enrollmentTokensSummary} onSuccess={showSuccess} onError={showError} />
				{#if enrollmentTokenError}
					<Callout tone="danger" title="Unable to load enrollment tokens" message={enrollmentTokenError}>
						<div class="mt-2">
							<Button variant="primary" size="sm" onclick={() => loadAllSettings()}>Retry All</Button>
						</div>
					</Callout>
				{/if}

				{#if canManageGlobalSettings}
					<DangerZone onSuccess={showSuccess} onError={showError} />
				{/if}
			</div>

			<!-- MCP Access tab -->
		{:else if activeTab === 'mcp-access'}
			<McpAccessTab />

			<!-- Plugin Configs tab -->
		{:else if activeTab === 'plugin-configs'}
			<PluginConfigsTab />

			<!-- Scheduler tab -->
		{:else if activeTab === 'scheduler'}
			<SchedulerTab />

			<!-- Global Settings tab -->
		{:else if activeTab === 'global-settings'}
			<GlobalSettingsTab />

			<!-- Notification Rules tab -->
		{:else if activeTab === 'notification-rules'}
			<NotificationRulesSettings onSuccess={showSuccess} onError={showError} />

			<!-- Notification Log tab -->
		{:else if activeTab === 'notification-log'}
			<NotificationLogView />

			<!-- Instance Configuration tab -->
		{:else if activeTab === 'instance-config'}
			<InstanceConfigTab />
		{:else if showSurfaceSettingsTabs}
			{#each slotTabGroups as group (group.id)}
				{#if activeTab === group.id && !BUILTIN_TAB_IDS.has(group.id)}
					{#if group.surfaces.length === 1}
						<SectionCard>
							<SurfaceReadPanel surface={group.surfaces[0]} read={slotTabReads[group.surfaces[0].surface_id]} />
						</SectionCard>
					{:else}
						<div class="space-y-4">
							{#each group.surfaces as surface (surface.surface_id)}
								<SectionCard title={surface.label}>
									<SurfaceReadPanel {surface} read={slotTabReads[surface.surface_id]} />
								</SectionCard>
							{/each}
						</div>
					{/if}
				{/if}
			{/each}
		{/if}
		{#if builtinAppendGroup}
			<div class="space-y-4">
				{#each builtinAppendGroup.surfaces as surface (surface.surface_id)}
					<SectionCard title={surface.label}>
						<SurfaceReadPanel {surface} read={slotTabReads[surface.surface_id]} />
					</SectionCard>
				{/each}
			</div>
		{/if}
	</PageShell>
{/if}
