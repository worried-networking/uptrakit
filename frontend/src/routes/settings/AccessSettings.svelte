<script lang="ts">
	import { onMount } from 'svelte';
	import { SectionCard } from '$lib/components/ui';
	import { FormFieldRow, Checkbox, Input, RadioCardGroup } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';
	import { createFormDraft } from '$lib/forms/draft.svelte';
	import { getAccessSettings, updateAccessSettings } from '$lib/api';
	import type { UpdateAccessSettingsRequest } from '$lib/api';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	type AccessDraft = {
		mode: 'open' | 'invite' | 'closed';
		token: string;
		requireTokenForOidc: boolean;
		passwordAuthEnabled: boolean;
		twoFactorRequired: boolean;
	};

	const form = createFormDraft<AccessDraft>({
		mode: 'open',
		token: '',
		requireTokenForOidc: false,
		passwordAuthEnabled: true,
		twoFactorRequired: false
	});

	// Exclude token from dirty check — the server never returns it so it always starts empty.
	const isDirty = $derived(
		form.isFieldDirty('mode') ||
			form.isFieldDirty('requireTokenForOidc') ||
			form.isFieldDirty('passwordAuthEnabled') ||
			form.isFieldDirty('twoFactorRequired')
	);

	let loading = $state(true);
	let saving = $state(false);

	const modeOptions = [
		{ value: 'open' as const, label: 'Open', tooltip: 'Anyone can create an account.' },
		{ value: 'invite' as const, label: 'Invite Only', tooltip: 'Token required to register.' },
		{ value: 'closed' as const, label: 'Closed', tooltip: 'No new accounts allowed.' }
	];

	onMount(async () => {
		try {
			const { data } = await getAccessSettings();
			form.load({
				mode: data.mode,
				token: '',
				requireTokenForOidc: data.require_token_for_oidc,
				passwordAuthEnabled: data.password_auth_enabled,
				twoFactorRequired: data.two_factor_required
			});
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to load access settings');
		} finally {
			loading = false;
		}
	});

	async function save() {
		saving = true;
		try {
			const body: UpdateAccessSettingsRequest = {
				mode: form.draft.mode,
				password_auth_enabled: form.draft.passwordAuthEnabled,
				two_factor_required: form.draft.twoFactorRequired
			};
			if (form.draft.mode === 'invite') {
				if (form.draft.token) body.token = form.draft.token;
				body.require_token_for_oidc = form.draft.requireTokenForOidc;
			}
			const { data } = await updateAccessSettings({ body });
			form.commit({
				mode: data.mode,
				token: '',
				requireTokenForOidc: data.require_token_for_oidc,
				passwordAuthEnabled: data.password_auth_enabled,
				twoFactorRequired: data.two_factor_required
			});
			onSuccess('Access settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save access settings');
		} finally {
			saving = false;
		}
	}
</script>

<SectionCard title="Registration & Authentication" description="Control who can register and how users authenticate.">
	{#if loading}
		<p class="text-[var(--text-secondary)]">Loading...</p>
	{:else}
		<div class="space-y-4">
			<FormFieldRow label="Registration Mode" dirty={form.isFieldDirty('mode')}>
				<RadioCardGroup
					name="registration-mode"
					value={form.draft.mode}
					options={modeOptions}
					onchange={(v) => form.update('mode', v)}
					disabled={saving}
				/>
			</FormFieldRow>

			{#if form.draft.mode === 'invite'}
				<FormFieldRow
					label="Registration Token"
					inputId="reg-token"
					hint="Set a new token. Leave blank to keep the current token."
					dirty={false}
				>
					<Input
						id="reg-token"
						type="text"
						placeholder="Enter a new registration token"
						bind:value={form.draft.token}
						disabled={saving}
					/>
				</FormFieldRow>

				<FormFieldRow label="OIDC First Login" dirty={form.isFieldDirty('requireTokenForOidc')}>
					<label class="flex items-center gap-3">
						<Checkbox id="oidc-first-login" bind:checked={form.draft.requireTokenForOidc} disabled={saving} />
						<span>Require token for OIDC first login</span>
					</label>
				</FormFieldRow>
			{/if}

			<FormFieldRow label="Password Authentication" dirty={form.isFieldDirty('passwordAuthEnabled')}>
				<label class="flex items-center gap-3">
					<Checkbox id="password-auth" bind:checked={form.draft.passwordAuthEnabled} disabled={saving} />
					<span>Enable password-based login</span>
				</label>
			</FormFieldRow>

			<FormFieldRow label="Require Two-Factor Auth" dirty={form.isFieldDirty('twoFactorRequired')}>
				<label class="flex items-center gap-3">
					<Checkbox id="two-factor-required" bind:checked={form.draft.twoFactorRequired} disabled={saving} />
					<span>Require 2FA for all users</span>
				</label>
			</FormFieldRow>

			<div class="flex items-center gap-2">
				<Button variant="primary" disabled={!isDirty || saving} loading={saving} onclick={save}>Save</Button>
				{#if isDirty}
					<Button variant="ghost" disabled={saving} onclick={() => form.discard()}>Discard</Button>
				{/if}
			</div>
		</div>
	{/if}
</SectionCard>
