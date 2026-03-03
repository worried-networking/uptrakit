<script lang="ts">
	import { getUser } from '$lib/auth.svelte';
	import { goto } from '$app/navigation';
	import { onDestroy } from 'svelte';
	import { getCombinedSettings, getOidcProviders, getMqttClients } from '$lib/api';
	import type {
		RegistrationSettings as RegistrationSettingsData,
		AuthenticationSettings as AuthenticationSettingsData,
		AgentCertificateSettings as AgentCertSettingsData,
		EnrollmentTokensSummary,
		MqttClientResponse,
		OidcProviderResponse
	} from '$lib/types';
	import { Permission } from '$lib/types';
	import SystemServicesSettings from './SystemServicesSettings.svelte';
	import { showSuccess, showError } from '$lib/notifications.svelte';

	import RegistrationSettings from './RegistrationSettings.svelte';
	import AuthenticationSettings from './AuthenticationSettings.svelte';
	import MqttClientsSettings from './MqttClientsSettings.svelte';
	import OidcProvidersSettings from './OidcProvidersSettings.svelte';
	import AgentCertificateSettings from './AgentCertificateSettings.svelte';
	import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';

	let loading: boolean = $state(true);

	let registrationSettings: RegistrationSettingsData | undefined = $state(undefined);
	let authSettings: AuthenticationSettingsData | undefined = $state(undefined);
	let mqttClients: MqttClientResponse[] | undefined = $state(undefined);
	let oidcProviders: OidcProviderResponse[] | undefined = $state(undefined);
	let agentCertSettings: AgentCertSettingsData | undefined = $state(undefined);
	let enrollmentTokensSummary: EnrollmentTokensSummary | undefined = $state(undefined);

	// Not reactive — only used for setTimeout cleanup.
	let mqttPollHandle: ReturnType<typeof setTimeout> | null = null;

	let registrationError: string | null = $state(null);
	let authenticationError: string | null = $state(null);
	let mqttClientsError: string | null = $state(null);
	let oidcProvidersError: string | null = $state(null);
	let agentCertificateError: string | null = $state(null);
	let enrollmentTokenError: string | null = $state(null);
	let mqttPollAttempt: number = $state(0);
	const initialMqttPollDelay = 10_000; // 10 seconds
	const maxMqttPollDelay = 5 * 60 * 1000; // 5 minutes

	const canManageSettings = $derived(getUser()?.permissions.includes(Permission.ManageSettings) ?? false);
	const canManageSystemServices = $derived(getUser()?.permissions.includes(Permission.ManageSystemServices) ?? false);

	$effect(() => {
		if (getUser() && !canManageSettings) {
			goto('/');
		}
	});

	$effect(() => {
		if (canManageSettings) {
			loadAllSettings();
			startMqttPolling();
		}
	});

	onDestroy(() => {
		stopMqttPolling();
	});

	function startMqttPolling() {
		if (mqttPollHandle) return; // Polling already active

		const poll = async () => {
			try {
				const clients = await getMqttClients();
				mqttClients = clients;
				mqttPollAttempt = 0; // Reset on success
				scheduleNextPoll(initialMqttPollDelay);
			} catch {
				// Suppress polling errors to avoid notification spam.
				mqttPollAttempt++;
				const baseDelay = Math.min(initialMqttPollDelay * Math.pow(2, mqttPollAttempt - 1), maxMqttPollDelay);
				const delay = baseDelay * (0.5 + Math.random() * 0.5); // Jitter to prevent thundering herd
				scheduleNextPoll(delay);
			}
		};

		const scheduleNextPoll = (delay: number) => {
			mqttPollHandle = setTimeout(poll, delay);
		};

		// Start the first poll after initial delay
		scheduleNextPoll(initialMqttPollDelay);
	}

	function stopMqttPolling() {
		if (mqttPollHandle) {
			clearTimeout(mqttPollHandle);
			mqttPollHandle = null;
			mqttPollAttempt = 0; // Reset attempt count when stopping
		}
	}

	async function loadAllSettings() {
		loading = true;
		// Clear previous errors
		registrationError = null;
		authenticationError = null;
		mqttClientsError = null;
		oidcProvidersError = null;
		agentCertificateError = null;
		enrollmentTokenError = null;

		const results = await Promise.allSettled([
			getCombinedSettings(), // results[0]
			getOidcProviders(), // results[1]
			getMqttClients() // results[2]
		]);

		// Combined Settings
		if (results[0].status === 'fulfilled') {
			const combined = results[0].value;
			registrationSettings = combined.registration;
			authSettings = combined.authentication;
			agentCertSettings = combined.agent_certificates;
			enrollmentTokensSummary = combined.enrollment_tokens;
		} else {
			const msg = results[0].reason instanceof Error ? results[0].reason.message : 'Failed to load combined settings.';
			registrationError = msg;
			authenticationError = msg;
			agentCertificateError = msg;
			enrollmentTokenError = msg;
		}

		// OIDC Providers
		if (results[1].status === 'fulfilled') {
			oidcProviders = results[1].value;
		} else {
			oidcProvidersError =
				results[1].reason instanceof Error ? results[1].reason.message : 'Failed to load OIDC providers.';
		}

		// MQTT Clients
		if (results[2].status === 'fulfilled') {
			mqttClients = results[2].value;
		} else {
			mqttClientsError =
				results[2].reason instanceof Error ? results[2].reason.message : 'Failed to load MQTT clients.';
		}

		loading = false;
	}
</script>

{#if getUser() && canManageSettings}
	<h1 class="h1 mb-6">Settings</h1>

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
		<MqttClientsSettings clients={mqttClients} onSuccess={showSuccess} onError={showError} />
		{#if mqttClientsError}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{mqttClientsError}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
			</aside>
		{/if}
		<OidcProvidersSettings providers={oidcProviders} onSuccess={showSuccess} onError={showError} />
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
		{#if canManageSystemServices}
			<SystemServicesSettings onSuccess={showSuccess} onError={showError} />
		{/if}
	</div>
{/if}
