<script lang="ts">
	import {
		createEnrollmentToken,
		revokeEnrollmentToken
	} from '$lib/api';
	import type { EnrollmentTokenStatus } from '$lib/types';
	import { copyToClipboard } from '$lib/utils';

	let agentCopied: boolean = $state(false);
	let mqttCopied: boolean = $state(false);
	let sshAgentCopied: boolean = $state(false);

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let enrollmentConfigured: boolean = $state(false);
	let generatedToken: string | null = $state(null);
	let mqttEnrollmentConfigured: boolean = $state(false);
	let mqttGeneratedToken: string | null = $state(null);
	let sshAgentEnrollmentConfigured: boolean = $state(false);
	let sshAgentGeneratedToken: string | null = $state(null);

	export function loadAgent(status: EnrollmentTokenStatus) {
		enrollmentConfigured = status.configured;
	}

	export function loadMqtt(status: EnrollmentTokenStatus) {
		mqttEnrollmentConfigured = status.configured;
	}

	export function loadSshAgent(status: EnrollmentTokenStatus) {
		sshAgentEnrollmentConfigured = status.configured;
	}

	async function handleGenerateToken() {
		try {
			const res = await createEnrollmentToken('agent');
			generatedToken = res.token;
			enrollmentConfigured = true;
			onSuccess('Agent enrollment token generated.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to generate agent enrollment token');
		}
	}

	async function handleRevokeToken() {
		try {
			await revokeEnrollmentToken('agent');
			enrollmentConfigured = false;
			generatedToken = null;
			onSuccess('Agent enrollment token revoked.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to revoke agent enrollment token');
		}
	}

	async function handleGenerateMqttToken() {
		try {
			const res = await createEnrollmentToken('mqtt');
			mqttGeneratedToken = res.token;
			mqttEnrollmentConfigured = true;
			onSuccess('MQTT enrollment token generated.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to generate MQTT enrollment token');
		}
	}

	async function handleRevokeMqttToken() {
		try {
			await revokeEnrollmentToken('mqtt');
			mqttEnrollmentConfigured = false;
			mqttGeneratedToken = null;
			onSuccess('MQTT enrollment token revoked.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to revoke MQTT enrollment token');
		}
	}

	async function handleGenerateSshAgentToken() {
		try {
			const res = await createEnrollmentToken('ssh_agent');
			sshAgentGeneratedToken = res.token;
			sshAgentEnrollmentConfigured = true;
			onSuccess('SSH Agent enrollment token generated.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to generate SSH Agent enrollment token');
		}
	}

	async function handleRevokeSshAgentToken() {
		try {
			await revokeEnrollmentToken('ssh_agent');
			sshAgentEnrollmentConfigured = false;
			sshAgentGeneratedToken = null;
			onSuccess('SSH Agent enrollment token revoked.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to revoke SSH Agent enrollment token');
		}
	}
</script>

<!-- Agent Enrollment Token -->
<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">Agent Enrollment Token</h2>
	<div class="mb-4 flex items-center gap-3">
		<span>Status:</span>
		{#if enrollmentConfigured}
			<span class="badge preset-filled-success-500">Configured</span>
		{:else}
			<span class="badge preset-tonal">Not configured</span>
		{/if}
	</div>

	{#if generatedToken}
		<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
			<p class="font-bold">Copy it now — it will not be shown again</p>
			<div class="mt-2 flex items-start gap-2">
				<code class="flex-1 break-all">{generatedToken}</code>
				<button
					class="btn btn-sm preset-tonal flex-shrink-0"
					onclick={async () => {
						if (generatedToken && await copyToClipboard(generatedToken)) {
							agentCopied = true;
							setTimeout(() => { agentCopied = false; }, 2000);
						}
					}}
				>
					{agentCopied ? 'Copied!' : 'Copy'}
				</button>
			</div>
		</aside>
	{/if}

	<div class="flex gap-2">
		<button class="btn preset-filled-primary-500" onclick={handleGenerateToken}>
			{enrollmentConfigured ? 'Regenerate' : 'Generate'}
		</button>
		{#if enrollmentConfigured}
			<button class="btn preset-filled-error-500" onclick={handleRevokeToken}>
				Revoke
			</button>
		{/if}
	</div>
</div>

<!-- MQTT Enrollment Token -->
<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">MQTT Enrollment Token</h2>
	<p class="mb-4 text-sm text-surface-600 dark:text-surface-400">
		This token is used by MQTT services to register with the controller.
		It is separate from the agent enrollment token.
	</p>
	<div class="mb-4 flex items-center gap-3">
		<span>Status:</span>
		{#if mqttEnrollmentConfigured}
			<span class="badge preset-filled-success-500">Configured</span>
		{:else}
			<span class="badge preset-tonal">Not configured</span>
		{/if}
	</div>

	{#if mqttGeneratedToken}
		<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
			<p class="font-bold">Copy it now — it will not be shown again</p>
			<div class="mt-2 flex items-start gap-2">
				<code class="flex-1 break-all">{mqttGeneratedToken}</code>
				<button
					class="btn btn-sm preset-tonal flex-shrink-0"
					onclick={async () => {
						if (mqttGeneratedToken && await copyToClipboard(mqttGeneratedToken)) {
							mqttCopied = true;
							setTimeout(() => { mqttCopied = false; }, 2000);
						}
					}}
				>
					{mqttCopied ? 'Copied!' : 'Copy'}
				</button>
			</div>
		</aside>
	{/if}

	<div class="flex gap-2">
		<button class="btn preset-filled-primary-500" onclick={handleGenerateMqttToken}>
			{mqttEnrollmentConfigured ? 'Regenerate' : 'Generate'}
		</button>
		{#if mqttEnrollmentConfigured}
			<button class="btn preset-filled-error-500" onclick={handleRevokeMqttToken}>
				Revoke
			</button>
		{/if}
	</div>
</div>

<!-- SSH Agent Enrollment Token -->
<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">SSH Agent Enrollment Token</h2>
	<p class="mb-4 text-sm text-surface-600 dark:text-surface-400">
		This token is used by SSH agents to register with the controller.
		It is separate from the agent and MQTT enrollment tokens.
	</p>
	<div class="mb-4 flex items-center gap-3">
		<span>Status:</span>
		{#if sshAgentEnrollmentConfigured}
			<span class="badge preset-filled-success-500">Configured</span>
		{:else}
			<span class="badge preset-tonal">Not configured</span>
		{/if}
	</div>

	{#if sshAgentGeneratedToken}
		<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
			<p class="font-bold">Copy it now — it will not be shown again</p>
			<div class="mt-2 flex items-start gap-2">
				<code class="flex-1 break-all">{sshAgentGeneratedToken}</code>
				<button
					class="btn btn-sm preset-tonal flex-shrink-0"
					onclick={async () => {
						if (sshAgentGeneratedToken && await copyToClipboard(sshAgentGeneratedToken)) {
							sshAgentCopied = true;
							setTimeout(() => { sshAgentCopied = false; }, 2000);
						}
					}}
				>
					{sshAgentCopied ? 'Copied!' : 'Copy'}
				</button>
			</div>
		</aside>
	{/if}

	<div class="flex gap-2">
		<button class="btn preset-filled-primary-500" onclick={handleGenerateSshAgentToken}>
			{sshAgentEnrollmentConfigured ? 'Regenerate' : 'Generate'}
		</button>
		{#if sshAgentEnrollmentConfigured}
			<button class="btn preset-filled-error-500" onclick={handleRevokeSshAgentToken}>
				Revoke
			</button>
		{/if}
	</div>
</div>
