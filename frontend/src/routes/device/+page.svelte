<script lang="ts">
	import { page } from '$app/state';
	import { approveDeviceAuth } from '$lib/api';
	import { getLoading, getUser } from '$lib/auth.svelte';
	import { Callout } from '$lib/components/ui';
	import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';
	import Button from '$lib/components/Button.svelte';

	let error = $state('');
	let success = $state(false);
	let approving = $state(false);

	const DEVICE_CODE_PATTERN = /^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/;
	let rawCode = $derived(page.url.searchParams.get('code') || '');
	let code = $derived(DEVICE_CODE_PATTERN.test(rawCode) ? rawCode : '');
	let invalidCode = $derived(rawCode !== '' && code === '');
	let isLoggedIn = $derived(!!getUser());

	async function onApprove() {
		if (!code) return;
		error = '';
		approving = true;
		try {
			await approveDeviceAuth(code);
			success = true;
		} catch (err) {
			error = err instanceof Error ? err.message : 'Failed to authorize device';
		} finally {
			approving = false;
		}
	}
</script>

<PublicEntryShell
	eyebrow="Device approval"
	title="Authorize Device"
	subtitle="Confirm the code shown in your CLI to finish signing in."
>
	{#if getLoading()}
		<Callout tone="info" message="Loading your session..." />
	{:else if success}
		<Callout tone="success" title="Device approved" message="CLI session approved. You can close this tab." />
	{:else if invalidCode}
		<Callout
			tone="danger"
			title="Invalid code"
			message="Invalid device code format. Please use the link shown in your CLI."
		/>
	{:else if !code}
		<Callout
			tone="warning"
			title="Missing code"
			message="No device code provided. Please use the link shown in your CLI."
		/>
	{:else if !isLoggedIn}
		<Callout tone="info" message="You need to log in before you can authorize this device." />
		<Button
			variant="primary"
			href="/login?redirect=/device?code={encodeURIComponent(code)}"
			class="w-full justify-center"
		>
			Log in
		</Button>
	{:else}
		{#if error}
			<Callout tone="danger" title="Unable to authorize device" message={error} />
		{/if}

		<Callout
			tone="info"
			message="Your CLI is requesting access. Confirm the code below matches what is shown in your terminal."
		/>

		<div
			class="rounded-[4px] border border-[var(--border-subtle)] bg-[var(--bg-raised)] px-4 py-5 text-center"
			data-ui="device-code"
		>
			<span class="font-mono text-3xl font-semibold tracking-[0.32em] text-[var(--text-primary)]">{code}</span>
		</div>

		<Button
			variant="primary"
			type="button"
			class="w-full justify-center"
			disabled={approving}
			loading={approving}
			onclick={onApprove}
		>
			Approve
		</Button>
	{/if}
</PublicEntryShell>
