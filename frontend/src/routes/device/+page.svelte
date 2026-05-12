<script lang="ts">
	import { page } from '$app/state';
	import { approveDeviceAuth, denyDeviceAuth, lookupDeviceAuth, type DeviceLookup } from '$lib/api';
	import { getLoading, getUser } from '$lib/auth.svelte';
	import { Callout } from '$lib/components/ui';
	import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';
	import Button from '$lib/components/Button.svelte';

	let error = $state('');
	let success = $state(false);
	let denied = $state(false);
	let processing = $state(false);
	let lookup = $state<DeviceLookup | null>(null);

	const DEVICE_CODE_PATTERN = /^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/;
	let rawCode = $derived(page.url.searchParams.get('user_code') || '');
	let code = $derived(DEVICE_CODE_PATTERN.test(rawCode) ? rawCode : '');
	let invalidCode = $derived(rawCode !== '' && code === '');
	let isLoggedIn = $derived(!!getUser());

	$effect(() => {
		if (code && isLoggedIn && !lookup && !error) {
			lookupDeviceAuth(code)
				.then((r) => (lookup = r))
				.catch((err) => {
					error = err instanceof Error ? err.message : 'Lookup failed';
				});
		}
	});

	async function onApprove() {
		if (!code) return;
		error = '';
		processing = true;
		try {
			await approveDeviceAuth(code);
			success = true;
		} catch (err) {
			error = err instanceof Error ? err.message : 'Failed to authorize device';
		} finally {
			processing = false;
		}
	}

	async function onDeny() {
		if (!code) return;
		error = '';
		processing = true;
		try {
			await denyDeviceAuth(code);
			denied = true;
		} catch (err) {
			error = err instanceof Error ? err.message : 'Failed to deny device';
		} finally {
			processing = false;
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
	{:else if denied}
		<Callout tone="warning" title="Device denied" message="CLI authorization denied. You can close this tab." />
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
			href="/login?redirect=/device?user_code={encodeURIComponent(code)}"
			class="w-full justify-center"
		>
			Log in
		</Button>
	{:else}
		{#if error}
			<Callout tone="danger" title="Unable to process device request" message={error} />
		{/if}

		{#if lookup?.client_name}
			<Callout
				tone="info"
				title="Approve sign-in"
				message="Approve sign-in from {lookup.client_name}? Confirm the code below matches what is shown in your terminal."
			/>
		{:else}
			<Callout
				tone="info"
				message="Your CLI is requesting access. Confirm the code below matches what is shown in your terminal."
			/>
		{/if}

		<div
			class="rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-raised)] px-4 py-5 text-center"
			data-ui="device-code"
		>
			<span class="font-mono text-device-code font-semibold tracking-device-code text-[var(--text-primary)]"
				>{code}</span
			>
		</div>

		<div class="flex gap-3">
			<Button
				variant="primary"
				type="button"
				class="flex-1 justify-center"
				disabled={processing}
				loading={processing}
				onclick={onApprove}
			>
				Approve
			</Button>
			<Button
				variant="secondary"
				type="button"
				class="flex-1 justify-center"
				disabled={processing}
				loading={processing}
				onclick={onDeny}
			>
				Deny
			</Button>
		</div>
	{/if}
</PublicEntryShell>
