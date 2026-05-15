<script lang="ts">
	import { updateAuthenticationSettings } from '$lib/api';
	import type { AuthenticationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { SectionCard } from '$lib/components/ui';
	import { FormFieldRow, Checkbox } from '$lib/components/forms';
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
	let twoFactorRequired: boolean = $state(false);
	let isSaving: boolean = $state(false);

	$effect(() => {
		if (settings) {
			passwordAuthEnabled = settings.password_auth_enabled;
			twoFactorRequired = settings.two_factor_required;
		}
	});

	async function saveAuthentication() {
		isSaving = true;
		try {
			const res = await updateAuthenticationSettings({
				password_auth_enabled: passwordAuthEnabled,
				two_factor_required: twoFactorRequired
			});
			passwordAuthEnabled = res.password_auth_enabled;
			twoFactorRequired = res.two_factor_required;
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
		<p class="text-[var(--text-secondary)]">Loading...</p>
	{:else}
		<div class="space-y-4">
			<FormFieldRow label="Password Authentication" inputId="password-auth-enabled">
				<label class="flex items-center gap-3">
					<Checkbox id="password-auth-enabled" bind:checked={passwordAuthEnabled} />
					<span>Enable password authentication</span>
				</label>
			</FormFieldRow>
			<FormFieldRow label="Require Two-Factor Authentication" inputId="two-factor-required">
				<label class="flex items-center gap-3">
					<Checkbox id="two-factor-required" bind:checked={twoFactorRequired} />
					<span>Require all password-authenticated users to enroll in 2FA</span>
				</label>
			</FormFieldRow>
			<div class="flex items-center gap-2">
				<Button variant="primary" loading={isSaving} disabled={!getIsOnline()} onclick={saveAuthentication}>Save</Button
				>
				{#if !getIsOnline()}<span class="text-[var(--color-warning)] text-sm">Offline</span>{/if}
			</div>
		</div>
	{/if}
</SectionCard>
