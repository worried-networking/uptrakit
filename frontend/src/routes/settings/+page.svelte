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
	import { Permission, hasAnyPermission } from '$lib/types';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import { getGroupedTabExtensions } from '$lib/extensions.svelte';

	import RegistrationSettings from './RegistrationSettings.svelte';
	import AuthenticationSettings from './AuthenticationSettings.svelte';
	import OidcProvidersSettings from './OidcProvidersSettings.svelte';
	import AgentCertificateSettings from './AgentCertificateSettings.svelte';
	import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';
	import PluginConfigsTab from './PluginConfigsTab.svelte';
	import SchedulerTab from './SchedulerTab.svelte';
	import GlobalSettingsTab from './GlobalSettingsTab.svelte';
	import ExtensionTabContent from '$lib/components/extensions/ExtensionTabContent.svelte';
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
	const groupedTabs = $derived(getGroupedTabExtensions('settings'));
	const tabExtensions = $derived([...groupedTabs.ungrouped]);
	const tabGroupNames = $derived([...groupedTabs.groups.keys()]);

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
		const isBuiltinAccessible =
			(activeTab === 'general' && canManageSettings) ||
			(activeTab === 'plugin-configs' && (canViewSoftware || canViewTypeSettings)) ||
			(activeTab === 'scheduler' && canManageSoftware) ||
			(activeTab === 'global-settings' && canManageGlobalSettings) ||
			(activeTab === 'notification-rules' && canViewNotifications) ||
			(activeTab === 'notification-log' && canViewNotifications);
		const isExtAccessible =
			tabExtensions.some((e) => e.id === activeTab) || tabGroupNames.some((g) => `group:${g}` === activeTab);
		if (!isBuiltinAccessible && !isExtAccessible) {
			if (canManageSettings) activeTab = 'general';
			else if (canViewSoftware || canViewTypeSettings) activeTab = 'plugin-configs';
			else if (canManageSoftware) activeTab = 'scheduler';
			else if (canManageGlobalSettings) activeTab = 'global-settings';
			else if (canViewNotifications) activeTab = 'notification-rules';
			else if (tabGroupNames.length > 0) activeTab = `group:${tabGroupNames[0]}`;
			else if (tabExtensions.length > 0) activeTab = tabExtensions[0].id;
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
	<h1 class="h1 mb-6">Settings</h1>

	<!-- Top-level tab bar -->
	<div class="mb-6 flex gap-1 flex-wrap">
		{#if canManageSettings}
			<button
				class="btn btn-sm {activeTab === 'general' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = 'general')}
			>
				General
			</button>
		{/if}
		{#if canViewSoftware || canViewTypeSettings}
			<button
				class="btn btn-sm {activeTab === 'plugin-configs' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = 'plugin-configs')}
			>
				Plugin Configs
			</button>
		{/if}
		{#if canManageSoftware}
			<button
				class="btn btn-sm {activeTab === 'scheduler' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = 'scheduler')}
			>
				Scheduler
			</button>
		{/if}
		{#if canManageGlobalSettings}
			<button
				class="btn btn-sm {activeTab === 'global-settings' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = 'global-settings')}
			>
				Global Settings
			</button>
		{/if}
		{#if canViewNotifications}
			<button
				class="btn btn-sm {activeTab === 'notification-rules' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = 'notification-rules')}
			>
				Notification Rules
			</button>
			<button
				class="btn btn-sm {activeTab === 'notification-log' ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = 'notification-log')}
			>
				Notification Log
			</button>
		{/if}
		{#each tabGroupNames as group (group)}
			<button
				class="btn btn-sm {activeTab === `group:${group}` ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = `group:${group}`)}
			>
				{group}
			</button>
		{/each}
		{#each tabExtensions as ext (ext.id)}
			<button
				class="btn btn-sm {activeTab === ext.id ? 'preset-filled-primary-500' : 'preset-tonal'}"
				onclick={() => (activeTab = ext.id)}
			>
				{ext.label}
			</button>
		{/each}
	</div>

	<!-- General tab -->
	{#if activeTab === 'general'}
		{#if loading}
			<div class="card mb-6 p-8 text-center">
				<p>Loading settings...</p>
			</div>
		{/if}

		<div aria-busy={loading} class:opacity-50={loading}>
			<RegistrationSettings settings={registrationSettings} onSuccess={showSuccess} onError={showError} />
			{#if registrationError}
				<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
					<p>{registrationError}</p>
					<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
				</aside>
			{/if}
			<AuthenticationSettings settings={authSettings} onSuccess={showSuccess} onError={showError} />
			{#if authenticationError}
				<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
					<p>{authenticationError}</p>
					<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
				</aside>
			{/if}
			<OidcProvidersSettings
				providers={oidcProviders}
				{multiTenancyEnabled}
				onSuccess={showSuccess}
				onError={showError}
			/>
			{#if oidcProvidersError}
				<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
					<p>{oidcProvidersError}</p>
					<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
				</aside>
			{/if}
			<AgentCertificateSettings settings={agentCertSettings} onSuccess={showSuccess} onError={showError} />
			{#if agentCertificateError}
				<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
					<p>{agentCertificateError}</p>
					<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
				</aside>
			{/if}
			<EnrollmentTokenSettings summary={enrollmentTokensSummary} onSuccess={showSuccess} onError={showError} />
			{#if enrollmentTokenError}
				<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
					<p>{enrollmentTokenError}</p>
					<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
				</aside>
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

		<!-- Extension group tabs -->
	{:else if activeTab.startsWith('group:')}
		{@const groupName = activeTab.slice('group:'.length)}
		{@const groupExts = groupedTabs.groups.get(groupName) ?? []}
		{#each groupExts as ext (ext.id)}
			<div class="card mb-6 p-6">
				<h2 class="h3 mb-4">{ext.label}</h2>
				<ExtensionTabContent extension={ext} />
			</div>
		{/each}

		<!-- Ungrouped extension tabs -->
	{:else}
		{#each tabExtensions as ext (ext.id)}
			{#if activeTab === ext.id}
				<ExtensionTabContent extension={ext} />
			{/if}
		{/each}
	{/if}
{/if}
