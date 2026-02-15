<script lang="ts">
	import { user } from '$lib/auth';
	import { goto } from '$app/navigation';
	import { onDestroy, onMount, tick } from 'svelte';
	import {
		getCombinedSettings,
		getOidcProviders,
		getMqttClients
	} from '$lib/api';
	import { Permission } from '$lib/types';
	import { showSuccess, showError } from '$lib/notifications.svelte';

	import RegistrationSettings from './RegistrationSettings.svelte';
	import AuthenticationSettings from './AuthenticationSettings.svelte';
	import MqttClientsSettings from './MqttClientsSettings.svelte';
	import OidcProvidersSettings from './OidcProvidersSettings.svelte';
	import AgentCertificateSettings from './AgentCertificateSettings.svelte';
	import EnrollmentTokenSettings from './EnrollmentTokenSettings.svelte';

	let loading: boolean = $state(true);
	let refsReady: boolean = $state(false);

	let registrationRef: RegistrationSettings = $state(undefined!);
	let authenticationRef: AuthenticationSettings = $state(undefined!);
	let mqttClientsRef: MqttClientsSettings = $state(undefined!);
	let oidcProvidersRef: OidcProvidersSettings = $state(undefined!);
	let agentCertificateRef: AgentCertificateSettings = $state(undefined!);
	let enrollmentTokenRef: EnrollmentTokenSettings = $state(undefined!);
	let mqttPollHandle: ReturnType<typeof setTimeout> | null = $state(null);

	let registrationError: string | null = $state(null);
	let authenticationError: string | null = $state(null);
	let mqttClientsError: string | null = $state(null);
	let oidcProvidersError: string | null = $state(null);
	let agentCertificateError: string | null = $state(null);
	let enrollmentTokenError: string | null = $state(null);
	let mqttPollAttempt: number = $state(0);
	const initialMqttPollDelay = 10_000; // 10 seconds
	const maxMqttPollDelay = 5 * 60 * 1000; // 5 minutes

	const canManageSettings = $derived($user?.permissions.includes(Permission.ManageSettings) ?? false);

	$effect(() => {
		if ($user && !canManageSettings) {
			goto('/');
		}
	});

	$effect(() => {
		if (canManageSettings) {
			if (refsReady) {
				loadAllSettings();
				startMqttPolling();
			}
		}
	});

	onMount(async () => {
		await tick();
		refsReady = true;
	});

	onDestroy(() => {
		stopMqttPolling();
	});

	function startMqttPolling() {
		if (mqttPollHandle) return; // Polling already active

		const poll = async () => {
			try {
				const clients = await getMqttClients();
				mqttClientsRef.load(clients);
				mqttPollAttempt = 0; // Reset on success
				scheduleNextPoll(initialMqttPollDelay);
			} catch {
				// Suppress polling errors to avoid notification spam.
				mqttPollAttempt++;
				const delay = Math.min(
					initialMqttPollDelay * Math.pow(2, mqttPollAttempt - 1), // Exponential backoff
					maxMqttPollDelay
				);
				scheduleNextPoll(delay);
			}
		};

		const scheduleNextPoll = (delay: number) => {
			mqttPollHandle = setTimeout(poll, delay);
		};

		// Start the first poll immediately or after initial delay
		scheduleNextPoll(initialMqttPollDelay);
	}

	function stopMqttPolling() {
		if (mqttPollHandle) {
			clearTimeout(mqttPollHandle); // Use clearTimeout instead of clearInterval
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
			getOidcProviders(),     // results[1]
			getMqttClients()        // results[2]
		]);

		// Combined Settings
		if (results[0].status === 'fulfilled') {
			const combined = results[0].value;
			registrationRef.load(combined.registration);
			authenticationRef.load(combined.authentication);
			agentCertificateRef.load(combined.agent_certificates);
			enrollmentTokenRef.loadAgent(combined.enrollment_tokens.agent);
			enrollmentTokenRef.loadMqtt(combined.enrollment_tokens.mqtt);
		} else {
			// This error affects multiple refs, so set specific errors for each
			const msg = results[0].reason instanceof Error ? results[0].reason.message : 'Failed to load combined settings.';
			registrationError = msg;
			authenticationError = msg;
			agentCertificateError = msg;
			enrollmentTokenError = msg;
		}

		// OIDC Providers
		if (results[1].status === 'fulfilled') {
			oidcProvidersRef.load(results[1].value);
		} else {
			oidcProvidersError = results[1].reason instanceof Error ? results[1].reason.message : 'Failed to load OIDC providers.';
		}

		// MQTT Clients
		if (results[2].status === 'fulfilled') {
			mqttClientsRef.load(results[2].value);
		} else {
			mqttClientsError = results[2].reason instanceof Error ? results[2].reason.message : 'Failed to load MQTT clients.';
		}

		loading = false;
	}
</script>

{#if $user && canManageSettings}
	<h1 class="h1 mb-6">Settings</h1>

	{#if loading}
		<div class="card mb-6 p-8 text-center">
			<p>Loading settings...</p>
		</div>
	{/if}

	<div aria-busy={loading} class:opacity-50={loading}>
		<RegistrationSettings bind:this={registrationRef} onSuccess={showSuccess} onError={showError} />
		{#if registrationError}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{registrationError}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
			</aside>
		{/if}
		<AuthenticationSettings bind:this={authenticationRef} onSuccess={showSuccess} onError={showError} />
		{#if authenticationError}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{authenticationError}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
			</aside>
		{/if}
		<MqttClientsSettings bind:this={mqttClientsRef} onSuccess={showSuccess} onError={showError} />
		{#if mqttClientsError}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{mqttClientsError}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
			</aside>
		{/if}
		<OidcProvidersSettings bind:this={oidcProvidersRef} onSuccess={showSuccess} onError={showError} />
		{#if oidcProvidersError}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{oidcProvidersError}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
			</aside>
		{/if}
		<AgentCertificateSettings bind:this={agentCertificateRef} onSuccess={showSuccess} onError={showError} />
		{#if agentCertificateError}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{agentCertificateError}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
			</aside>
		{/if}
		<EnrollmentTokenSettings bind:this={enrollmentTokenRef} onSuccess={showSuccess} onError={showError} />
		{#if enrollmentTokenError}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{enrollmentTokenError}</p>
				<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadAllSettings()}>Retry All</button>
			</aside>
		{/if}
	</div>
{/if}
