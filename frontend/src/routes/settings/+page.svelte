<script lang="ts">
	import { getUser } from '$lib/auth.svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getCombinedSettings, getOidcProviders } from '$lib/api';
	import type {
		RegistrationSettings as RegistrationSettingsData,
		AuthenticationSettings as AuthenticationSettingsData,
		AgentCertificateSettings as AgentCertSettingsData,
		EnrollmentTokensSummary,
		OidcProviderResponse
	} from '$lib/types';
	import { Permission, hasAnyPermission, hasPermissionValue } from '$lib/types';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import {
		getSurfaceReadLoading,
		getSurfaceReadModel,
		getSurfaceReadRequested,
		getSurfaceRegistryLoaded,
		getSurfaceRuntimeStatus,
		getSurfacesBySlot,
		loadSurfaceReadModels
	} from '$lib/surfaces/registry.svelte';
	import { filterSurfacesByPermission, isSurfaceTabPending } from '$lib/surfaces/read-model';
	import { Callout, PageShell, SectionCard, TabStrip, type TabStripItem } from '$lib/components/ui';

	import RegistrationSettings from './RegistrationSettings.svelte';
	import AuthenticationSettings from './AuthenticationSettings.svelte';
	import OidcProvidersSettings from './OidcProvidersSettings.svelte';
	import AgentCertificateSettings from './AgentCertificateSettings.svelte';
	import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';
	import PluginConfigsTab from './PluginConfigsTab.svelte';
	import SchedulerTab from './SchedulerTab.svelte';
	import GlobalSettingsTab from './GlobalSettingsTab.svelte';
	import NotificationRulesSettings from './NotificationRulesSettings.svelte';
	import NotificationLogView from './NotificationLogView.svelte';
	import DangerZone from './DangerZone.svelte';

	// ── Permissions ─────────────────────────────────────────────────────
	const canManageSettings = $derived(
		hasAnyPermission(
			getUser(),
			Permission.ManageAuthSettings,
			Permission.ManageEnrollmentTokens,
			Permission.ManageAgentCerts
		)
	);
	const canViewSoftware = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canViewTypeSettings = $derived(
		hasAnyPermission(getUser(), Permission.ViewSettings, Permission.ManageGlobalSettings)
	);
	const canManageSoftware = $derived(
		hasAnyPermission(
			getUser(),
			Permission.CreateSoftware,
			Permission.UpdateSoftware,
			Permission.DeleteSoftware,
			Permission.TriggerChecks,
			Permission.TriggerUpdates,
			Permission.ManageScheduler
		)
	);
	const canManageGlobalSettings = $derived(getUser()?.permissions.includes(Permission.ManageGlobalSettings) ?? false);
	const canViewNotifications = $derived(getUser()?.permissions.includes(Permission.ViewNotifications) ?? false);
	const hasAnyTabPermission = $derived(
		canManageSettings ||
			canViewSoftware ||
			canViewTypeSettings ||
			canManageSoftware ||
			canManageGlobalSettings ||
			canViewNotifications
	);

	// ── Tab state ────────────────────────────────────────────────────────
	const slotTabSurfaces = $derived(
		filterSurfacesByPermission(getSurfacesBySlot('settings.tabs'), (requiredPermission) =>
			hasPermissionValue(getUser(), requiredPermission)
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
	const showSurfaceSettingsTabs = $derived(getSurfaceRuntimeStatus().active && slotTabSurfaces.length > 0);
	const tabItems = $derived.by<TabStripItem[]>(() => {
		const items: TabStripItem[] = [];
		if (canManageSettings) {
			items.push({ id: 'general', label: 'General' });
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
		if (showSurfaceSettingsTabs) {
			for (const surface of slotTabSurfaces) {
				items.push({ id: surface.surface_id, label: surface.label });
			}
		}
		return items;
	});

	let activeTab: string = $state(page.url.searchParams.get('tab') ?? 'general');

	// Redirect if no permissions at all
	$effect(() => {
		if (getUser() && !hasAnyTabPermission) {
			goto('/');
		}
	});

	// Correct activeTab if it's not accessible
	$effect(() => {
		const user = getUser();
		if (!user) return;
		const surfaceRuntimeActive = getSurfaceRuntimeStatus().active;
		const surfaceRegistryLoaded = getSurfaceRegistryLoaded();
		const isBuiltinAccessible =
			(activeTab === 'general' && canManageSettings) ||
			(activeTab === 'plugin-configs' && (canViewSoftware || canViewTypeSettings)) ||
			(activeTab === 'scheduler' && canManageSoftware) ||
			(activeTab === 'global-settings' && canManageGlobalSettings) ||
			(activeTab === 'notification-rules' && canViewNotifications) ||
			(activeTab === 'notification-log' && canViewNotifications);
		const isSurfaceAccessible =
			showSurfaceSettingsTabs && slotTabSurfaces.some((surface) => surface.surface_id === activeTab);
		const isPendingSurfaceTab = isSurfaceTabPending({
			rolloutActive: surfaceRuntimeActive,
			activeTab,
			slotSurfaces: slotTabSurfaces,
			readBySurface: slotTabReads,
			isReadRequested: getSurfaceReadRequested(activeTab),
			isReadLoading: getSurfaceReadLoading(activeTab)
		});
		if (!surfaceRegistryLoaded && !isBuiltinAccessible) {
			return;
		}
		if (!isBuiltinAccessible && !isSurfaceAccessible && !isPendingSurfaceTab) {
			if (canManageSettings) activeTab = 'general';
			else if (canViewSoftware || canViewTypeSettings) activeTab = 'plugin-configs';
			else if (canManageSoftware) activeTab = 'scheduler';
			else if (canManageGlobalSettings) activeTab = 'global-settings';
			else if (canViewNotifications) activeTab = 'notification-rules';
			else if (showSurfaceSettingsTabs && slotTabSurfaces.length > 0) activeTab = slotTabSurfaces[0].surface_id;
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

	let registrationSettings: RegistrationSettingsData | undefined = $state(undefined);
	let authSettings: AuthenticationSettingsData | undefined = $state(undefined);
	let oidcProviders: OidcProviderResponse[] | undefined = $state(undefined);
	let agentCertSettings: AgentCertSettingsData | undefined = $state(undefined);
	let enrollmentTokensSummary: EnrollmentTokensSummary | undefined = $state(undefined);
	let multiTenancyEnabled: boolean = $state(false);

	let registrationError: string | null = $state(null);
	let authenticationError: string | null = $state(null);
	let oidcProvidersError: string | null = $state(null);
	let agentCertificateError: string | null = $state(null);
	let enrollmentTokenError: string | null = $state(null);

	$effect(() => {
		if (canManageSettings) {
			loadAllSettings();
		}
	});

	$effect(() => {
		if (!getSurfaceRuntimeStatus().active || slotTabSurfaces.length === 0) {
			return;
		}
		void loadSurfaceReadModels(slotTabSurfaces.map((surface) => surface.surface_id));
	});

	async function loadAllSettings() {
		loading = true;
		registrationError = null;
		authenticationError = null;
		oidcProvidersError = null;
		agentCertificateError = null;
		enrollmentTokenError = null;

		const results = await Promise.allSettled([getCombinedSettings(), getOidcProviders()]);

		if (results[0].status === 'fulfilled') {
			const combined = results[0].value;
			registrationSettings = combined.registration;
			authSettings = combined.authentication;
			agentCertSettings = combined.agent_certificates;
			enrollmentTokensSummary = combined.enrollment_tokens;
			multiTenancyEnabled = combined.multi_tenancy_enabled;
		} else {
			const msg = results[0].reason instanceof Error ? results[0].reason.message : 'Failed to load combined settings.';
			registrationError = msg;
			authenticationError = msg;
			agentCertificateError = msg;
			enrollmentTokenError = msg;
		}

		if (results[1].status === 'fulfilled') {
			oidcProviders = results[1].value;
		} else {
			oidcProvidersError =
				results[1].reason instanceof Error ? results[1].reason.message : 'Failed to load OIDC providers.';
		}

		loading = false;
	}
</script>

{#if getUser() && hasAnyTabPermission}
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

			<div aria-busy={loading} class:opacity-50={loading}>
				<RegistrationSettings settings={registrationSettings} onSuccess={showSuccess} onError={showError} />
				{#if registrationError}
					<Callout tone="danger" title="Unable to load registration settings" message={registrationError}>
						<div class="mt-2">
							<button class="btn btn-sm preset-filled-primary-500" onclick={() => loadAllSettings()}>Retry All</button>
						</div>
					</Callout>
				{/if}
				<AuthenticationSettings settings={authSettings} onSuccess={showSuccess} onError={showError} />
				{#if authenticationError}
					<Callout tone="danger" title="Unable to load authentication settings" message={authenticationError}>
						<div class="mt-2">
							<button class="btn btn-sm preset-filled-primary-500" onclick={() => loadAllSettings()}>Retry All</button>
						</div>
					</Callout>
				{/if}
				<OidcProvidersSettings
					providers={oidcProviders}
					{multiTenancyEnabled}
					onSuccess={showSuccess}
					onError={showError}
				/>
				{#if oidcProvidersError}
					<Callout tone="danger" title="Unable to load OIDC providers" message={oidcProvidersError}>
						<div class="mt-2">
							<button class="btn btn-sm preset-filled-primary-500" onclick={() => loadAllSettings()}>Retry All</button>
						</div>
					</Callout>
				{/if}
				<AgentCertificateSettings settings={agentCertSettings} onSuccess={showSuccess} onError={showError} />
				{#if agentCertificateError}
					<Callout tone="danger" title="Unable to load certificate settings" message={agentCertificateError}>
						<div class="mt-2">
							<button class="btn btn-sm preset-filled-primary-500" onclick={() => loadAllSettings()}>Retry All</button>
						</div>
					</Callout>
				{/if}
				<EnrollmentTokenSettings summary={enrollmentTokensSummary} onSuccess={showSuccess} onError={showError} />
				{#if enrollmentTokenError}
					<Callout tone="danger" title="Unable to load enrollment tokens" message={enrollmentTokenError}>
						<div class="mt-2">
							<button class="btn btn-sm preset-filled-primary-500" onclick={() => loadAllSettings()}>Retry All</button>
						</div>
					</Callout>
				{/if}

				{#if canManageGlobalSettings}
					<DangerZone onSuccess={showSuccess} onError={showError} />
				{/if}
			</div>

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
		{:else if showSurfaceSettingsTabs}
			{#each slotTabSurfaces as surface (surface.surface_id)}
				{#if activeTab === surface.surface_id}
					<SectionCard title={surface.label}>
						<SurfaceReadPanel {surface} read={slotTabReads[surface.surface_id]} />
					</SectionCard>
				{/if}
			{/each}
		{/if}
	</PageShell>
{/if}
