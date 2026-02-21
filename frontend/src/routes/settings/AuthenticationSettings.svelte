<script lang="ts">
	import { updateAuthenticationSettings } from '$lib/api';
	import type { AuthenticationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: AuthenticationSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let passwordAuthEnabled: boolean = $state(true);

	$effect(() => {
		if (settings) {
			passwordAuthEnabled = settings.password_auth_enabled;
		}
	});

	async function saveAuthentication() {
		try {
			const res = await updateAuthenticationSettings({
				password_auth_enabled: passwordAuthEnabled
			});
			passwordAuthEnabled = res.password_auth_enabled;
			onSuccess('Authentication settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save authentication settings');
		}
	}
</script>

<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">Authentication</h2>
	{#if settings === undefined}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else}
		<label class="mb-4 flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={passwordAuthEnabled} />
			<span>Enable password authentication</span>
		</label>
		<div class="flex items-center gap-2">
			<button class="btn preset-filled-primary-500" onclick={saveAuthentication} disabled={!getIsOnline()}>
				Save
			</button>
			{#if !getIsOnline()}<span class="text-warning-500 text-sm">Offline</span>{/if}
		</div>
	{/if}
</div>
