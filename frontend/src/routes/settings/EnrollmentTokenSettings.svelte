<script lang="ts">
	import { createEnrollmentToken, revokeEnrollmentToken } from '$lib/api';
	import type { EnrollmentTokenStatus } from '$lib/types';
	import { copyToClipboard } from '$lib/utils';

	type TokenType = 'agent' | 'mqtt' | 'ssh_agent';

	interface TokenSection {
		type: TokenType;
		label: string;
		description?: string;
	}

	const tokenSections: TokenSection[] = [
		{ type: 'agent', label: 'Agent Enrollment Token' },
		{
			type: 'mqtt',
			label: 'MQTT Enrollment Token',
			description:
				'This token is used by MQTT services to register with the controller. It is separate from the agent enrollment token.'
		},
		{
			type: 'ssh_agent',
			label: 'SSH Agent Enrollment Token',
			description:
				'This token is used by SSH agents to register with the controller. It is separate from the agent and MQTT enrollment tokens.'
		}
	];

	let {
		agentStatus,
		mqttStatus,
		sshAgentStatus,
		onSuccess,
		onError
	}: {
		agentStatus: EnrollmentTokenStatus | undefined;
		mqttStatus: EnrollmentTokenStatus | undefined;
		sshAgentStatus: EnrollmentTokenStatus | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let configured: Record<TokenType, boolean> = $state({
		agent: false,
		mqtt: false,
		ssh_agent: false
	});

	let generatedTokens: Record<TokenType, string | null> = $state({
		agent: null,
		mqtt: null,
		ssh_agent: null
	});

	let copied: Record<TokenType, boolean> = $state({
		agent: false,
		mqtt: false,
		ssh_agent: false
	});

	$effect(() => {
		if (agentStatus) {
			configured.agent = agentStatus.configured;
		}
	});

	$effect(() => {
		if (mqttStatus) {
			configured.mqtt = mqttStatus.configured;
		}
	});

	$effect(() => {
		if (sshAgentStatus) {
			configured.ssh_agent = sshAgentStatus.configured;
		}
	});

	async function handleGenerate(section: TokenSection) {
		try {
			const res = await createEnrollmentToken(section.type);
			generatedTokens[section.type] = res.token;
			configured[section.type] = true;
			onSuccess(`${section.label.replace(' Enrollment Token', '')} enrollment token generated.`);
		} catch (e) {
			onError(
				e instanceof Error
					? e.message
					: `Failed to generate ${section.label.replace(' Enrollment Token', '').toLowerCase()} enrollment token`
			);
		}
	}

	async function handleRevoke(section: TokenSection) {
		try {
			await revokeEnrollmentToken(section.type);
			configured[section.type] = false;
			generatedTokens[section.type] = null;
			onSuccess(`${section.label.replace(' Enrollment Token', '')} enrollment token revoked.`);
		} catch (e) {
			onError(
				e instanceof Error
					? e.message
					: `Failed to revoke ${section.label.replace(' Enrollment Token', '').toLowerCase()} enrollment token`
			);
		}
	}

	async function handleCopy(type: TokenType) {
		const token = generatedTokens[type];
		if (token && (await copyToClipboard(token))) {
			copied[type] = true;
			setTimeout(() => {
				copied[type] = false;
			}, 2000);
		}
	}

	function isLoading(): boolean {
		return agentStatus === undefined || mqttStatus === undefined || sshAgentStatus === undefined;
	}
</script>

{#each tokenSections as section (section.type)}
	<div class="card mb-6 p-6">
		<h2 class="h3 mb-4">{section.label}</h2>
		{#if isLoading()}
			<p class="text-surface-600 dark:text-surface-400">Loading...</p>
		{:else}
			{#if section.description}
				<p class="mb-4 text-sm text-surface-600 dark:text-surface-400">
					{section.description}
				</p>
			{/if}
			<div class="mb-4 flex items-center gap-3">
				<span>Status:</span>
				{#if configured[section.type]}
					<span class="badge preset-filled-success-500">Configured</span>
				{:else}
					<span class="badge preset-tonal">Not configured</span>
				{/if}
			</div>

			{#if generatedTokens[section.type]}
				<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
					<p class="font-bold">Copy it now — it will not be shown again</p>
					<div class="mt-2 flex items-start gap-2">
						<code class="flex-1 break-all">{generatedTokens[section.type]}</code>
						<button class="btn btn-sm preset-tonal flex-shrink-0" onclick={() => handleCopy(section.type)}>
							{copied[section.type] ? 'Copied!' : 'Copy'}
						</button>
					</div>
				</aside>
			{/if}

			<div class="flex gap-2">
				<button class="btn preset-filled-primary-500" onclick={() => handleGenerate(section)}>
					{configured[section.type] ? 'Regenerate' : 'Generate'}
				</button>
				{#if configured[section.type]}
					<button class="btn preset-filled-error-500" onclick={() => handleRevoke(section)}> Revoke </button>
				{/if}
			</div>
		{/if}
	</div>
{/each}
