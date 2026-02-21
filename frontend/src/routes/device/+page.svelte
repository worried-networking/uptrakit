<script lang="ts">
	import { getUser, getLoading } from '$lib/auth.svelte';
	import { approveDeviceAuth } from '$lib/api';
	import { page } from '$app/stores';

	let error = $state('');
	let success = $state(false);
	let approving = $state(false);

	const DEVICE_CODE_PATTERN = /^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/;
	let rawCode = $derived($page.url.searchParams.get('code') || '');
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

<div class="card mx-auto mt-8 max-w-md p-8">
	<h2 class="h2 mb-6 text-center">Authorize Device</h2>

	{#if getLoading()}
		<p class="text-center text-surface-600 dark:text-surface-400">Loading...</p>
	{:else if success}
		<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
			<p>CLI session approved! You can close this tab.</p>
		</aside>
	{:else if invalidCode}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>Invalid device code format. Please use the link shown in your CLI.</p>
		</aside>
	{:else if !code}
		<aside class="mb-4 rounded-lg p-4 preset-filled-warning-500">
			<p>No device code provided. Please use the link shown in your CLI.</p>
		</aside>
	{:else if !isLoggedIn}
		<p class="mb-4 text-center text-surface-600 dark:text-surface-400">
			You need to log in before you can authorize a device.
		</p>
		<a href="/login?redirect=/device?code={encodeURIComponent(code)}" class="btn preset-filled-primary-500 w-full">
			Log in
		</a>
	{:else}
		{#if error}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{error}</p>
			</aside>
		{/if}

		<p class="mb-4 text-center text-surface-600 dark:text-surface-400">
			Your CLI is requesting access. Confirm the code below matches what is shown in your terminal.
		</p>

		<div class="mb-6 rounded-lg bg-surface-200 p-4 text-center dark:bg-surface-700">
			<span class="font-mono text-3xl font-bold tracking-widest">{code}</span>
		</div>

		<button type="button" class="btn preset-filled-primary-500 w-full" disabled={approving} onclick={onApprove}>
			{approving ? 'Authorizing...' : 'Approve'}
		</button>
	{/if}
</div>
