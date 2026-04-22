<script lang="ts">
	import { updateAuthenticationSettings } from '$lib/api';
	import type { AuthenticationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

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
	let isSaving: boolean = $state(false);

	$effect(() => {
		if (settings) {
			passwordAuthEnabled = settings.password_auth_enabled;
		}
	});

	async function saveAuthentication() {
		isSaving = true;
		try {
			const res = await updateAuthenticationSettings({
				password_auth_enabled: passwordAuthEnabled
			});
			passwordAuthEnabled = res.password_auth_enabled;
			onSuccess('Authentication settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save authentication settings');
		} finally {
			isSaving = false;
		}
	}
</script>

<SectionCard title="Authentication">
	{#if settings === undefined}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else}
		<div class="space-y-4">
			<FormFieldRow label="Password Authentication" inputId="password-auth-enabled">
				<label class="flex items-center gap-3">
					<input id="password-auth-enabled" class="checkbox" type="checkbox" bind:checked={passwordAuthEnabled} />
					<span>Enable password authentication</span>
				</label>
			</FormFieldRow>
			<div class="flex items-center gap-2">
				<Button variant="primary" loading={isSaving} disabled={!getIsOnline()} onclick={saveAuthentication}>Save</Button
				>
				{#if !getIsOnline()}<span class="text-warning-500 text-sm">Offline</span>{/if}
			</div>
		</div>
	{/if}
</SectionCard>
