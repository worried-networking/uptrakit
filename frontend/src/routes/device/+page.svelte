<script lang="ts">
	import { user, loading } from '$lib/auth';
	import { approveDeviceAuth } from '$lib/api';
	import { page } from '$app/stores';

	let error = $state('');
	let success = $state(false);
	let approving = $state(false);

	let code = $derived($page.url.searchParams.get('code') || '');
	let isLoggedIn = $derived(!!$user);

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

	{#if $loading}
		<p class="text-center text-surface-600-300-token">Loading...</p>
	{:else if success}
		<aside class="alert variant-filled-success mb-4">
			<div class="alert-message">
				<p>CLI session approved! You can close this tab.</p>
			</div>
		</aside>
	{:else if !code}
		<aside class="alert variant-filled-warning mb-4">
			<div class="alert-message">
				<p>No device code provided. Please use the link shown in your CLI.</p>
			</div>
		</aside>
	{:else if !isLoggedIn}
		<p class="mb-4 text-center text-surface-600-300-token">
			You need to log in before you can authorize a device.
		</p>
		<a
			href="/login?redirect=/device?code={encodeURIComponent(code)}"
			class="btn variant-filled-primary w-full"
		>
			Log in
		</a>
	{:else}
		{#if error}
			<aside class="alert variant-filled-error mb-4">
				<div class="alert-message">
					<p>{error}</p>
				</div>
			</aside>
		{/if}

		<p class="mb-4 text-center text-surface-600-300-token">
			Your CLI is requesting access. Confirm the code below matches what is shown in your terminal.
		</p>

		<div class="mb-6 rounded-lg bg-surface-200-700-token p-4 text-center">
			<span class="font-mono text-3xl font-bold tracking-widest">{code}</span>
		</div>

		<button
			type="button"
			class="btn variant-filled-primary w-full"
			disabled={approving}
			onclick={onApprove}
		>
			{approving ? 'Authorizing...' : 'Approve'}
		</button>
	{/if}
</div>
