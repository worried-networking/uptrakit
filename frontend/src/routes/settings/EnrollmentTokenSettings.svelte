<script lang="ts">
	import { createEnrollmentToken, revokeEnrollmentToken } from '$lib/api';
	import type { EnrollmentTokenStatus } from '$lib/types';
	import { copyToClipboard } from '$lib/utils';

	let {
		status,
		onSuccess,
		onError
	}: {
		status: EnrollmentTokenStatus | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let configured: boolean = $state(false);
	let generatedToken: string | null = $state(null);
	let copied: boolean = $state(false);

	$effect(() => {
		if (status) {
			configured = status.configured;
		}
	});

	async function handleGenerate() {
		try {
			const res = await createEnrollmentToken();
			generatedToken = res.token;
			configured = true;
			onSuccess('Enrollment token generated.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to generate enrollment token');
		}
	}

	async function handleRevoke() {
		try {
			await revokeEnrollmentToken();
			configured = false;
			generatedToken = null;
			onSuccess('Enrollment token revoked.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to revoke enrollment token');
		}
	}

	async function handleCopy() {
		if (generatedToken && (await copyToClipboard(generatedToken))) {
			copied = true;
			setTimeout(() => {
				copied = false;
			}, 2000);
		}
	}
</script>

<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">Service Enrollment Token</h2>
	{#if status === undefined}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else}
		<p class="mb-4 text-sm text-surface-600 dark:text-surface-400">
			A single enrollment token is used by all services (agents, MQTT bridges, SSH agents) to register with the
			controller.
		</p>
		<div class="mb-4 flex items-center gap-3">
			<span>Status:</span>
			{#if configured}
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
					<button class="btn btn-sm preset-tonal flex-shrink-0" onclick={handleCopy}>
						{copied ? 'Copied!' : 'Copy'}
					</button>
				</div>
			</aside>
		{/if}

		<div class="flex gap-2">
			<button class="btn preset-filled-primary-500" onclick={handleGenerate}>
				{configured ? 'Regenerate' : 'Generate'}
			</button>
			{#if configured}
				<button class="btn preset-filled-error-500" onclick={handleRevoke}> Revoke </button>
			{/if}
		</div>
	{/if}
</div>
