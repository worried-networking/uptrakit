<script lang="ts">
	import { user } from '$lib/auth';
	import { goto } from '$app/navigation';
	import { onMount, tick } from 'svelte';
	import {
		getCombinedSettings,
		getOidcProviders,
		getMqttClients
	} from '$lib/api';
	import { Permission } from '$lib/types';
	import { showSuccess, showError, clearError, getSuccessMessage, getErrorMessage } from '$lib/notifications.svelte';

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

	const canManageSettings = $derived($user?.permissions.includes(Permission.ManageSettings) ?? false);
	const successMessage = $derived(getSuccessMessage());
	const errorMessage = $derived(getErrorMessage());

	$effect(() => {
		if ($user && !canManageSettings) {
			goto('/');
		}
	});

	$effect(() => {
		if (canManageSettings) {
			if (refsReady) {
				loadAllSettings();
			}
		}
	});

	onMount(async () => {
		await tick();
		refsReady = true;
	});

	async function loadAllSettings() {
		loading = true;
		const results = await Promise.allSettled([
			getCombinedSettings(),
			getOidcProviders(),
			getMqttClients()
		]);

		if (results[0].status === 'fulfilled') {
			const combined = results[0].value;
			registrationRef.load(combined.registration);
			authenticationRef.load(combined.authentication);
			agentCertificateRef.load(combined.agent_certificates);
			enrollmentTokenRef.loadAgent(combined.enrollment_tokens.agent);
			enrollmentTokenRef.loadMqtt(combined.enrollment_tokens.mqtt);
		}
		if (results[1].status === 'fulfilled') oidcProvidersRef.load(results[1].value);
		if (results[2].status === 'fulfilled') mqttClientsRef.load(results[2].value);

		loading = false;
	}
</script>

{#if $user && canManageSettings}
	<h1 class="h1 mb-6">Settings</h1>

	{#if successMessage}
		<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
			<p>{successMessage}</p>
		</aside>
	{/if}

	{#if errorMessage}
		<aside class="mb-4 flex items-center justify-between rounded-lg p-4 preset-filled-error-500">
			<p>{errorMessage}</p>
			<button class="btn btn-sm preset-filled" onclick={clearError}>Dismiss</button>
		</aside>
	{/if}

	{#if loading}
		<div class="card mb-6 p-8 text-center">
			<p>Loading settings...</p>
		</div>
	{/if}

	<div aria-busy={loading} class:opacity-50={loading}>
		<RegistrationSettings bind:this={registrationRef} onSuccess={showSuccess} onError={showError} />
		<AuthenticationSettings bind:this={authenticationRef} onSuccess={showSuccess} onError={showError} />
		<MqttClientsSettings bind:this={mqttClientsRef} onSuccess={showSuccess} onError={showError} />
		<OidcProvidersSettings bind:this={oidcProvidersRef} onSuccess={showSuccess} onError={showError} />
		<AgentCertificateSettings bind:this={agentCertificateRef} onSuccess={showSuccess} onError={showError} />
		<EnrollmentTokenSettings bind:this={enrollmentTokenRef} onSuccess={showSuccess} onError={showError} />
	</div>
{/if}
