<script lang="ts">
	import { updateRegistrationSettings } from '$lib/api';
	import type { RegistrationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: RegistrationSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let regMode: 'open' | 'invite' | 'closed' = $state('open');
	let regToken: string = $state('');
	let regRequireTokenForOidc: boolean = $state(false);

	$effect(() => {
		if (settings) {
			regMode = settings.mode;
			regRequireTokenForOidc = settings.require_token_for_oidc;
		}
	});

	async function saveRegistration() {
		try {
			const data: { mode: 'open' | 'invite' | 'closed'; token?: string; require_token_for_oidc?: boolean } = {
				mode: regMode
			};
			if (regMode === 'invite' && regToken) {
				data.token = regToken;
			}
			if (regMode === 'invite') {
				data.require_token_for_oidc = regRequireTokenForOidc;
			}
			const res = await updateRegistrationSettings(data);
			regMode = res.mode;
			regRequireTokenForOidc = res.require_token_for_oidc;
			regToken = '';
			onSuccess('Registration settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save registration settings');
		}
	}
</script>

<SectionCard title="Registration">
	{#if settings === undefined}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else}
		<div class="space-y-4">
			<FormFieldRow label="Registration Mode" inputId="registration-mode">
				<select id="registration-mode" class="select" bind:value={regMode}>
					<option value="open">Open</option>
					<option value="invite">Invite Only</option>
					<option value="closed">Closed</option>
				</select>
			</FormFieldRow>

			{#if regMode === 'invite'}
				<FormFieldRow
					label="Registration Token"
					inputId="registration-token"
					hint="Set a new token for invite-only registration. Leave blank to keep the current token."
				>
					<input
						id="registration-token"
						class="input"
						type="text"
						placeholder="Enter a new registration token"
						bind:value={regToken}
					/>
				</FormFieldRow>

				<FormFieldRow
					label="OIDC First Login"
					inputId="registration-require-oidc-token"
					hint="When enabled, users signing in via OIDC for the first time must also provide the registration token."
				>
					<label class="flex items-center gap-3">
						<input
							id="registration-require-oidc-token"
							class="checkbox"
							type="checkbox"
							bind:checked={regRequireTokenForOidc}
						/>
						<span>Require registration token for OIDC users</span>
					</label>
				</FormFieldRow>
			{/if}

			<div class="flex items-center gap-2">
				<button class="btn preset-filled-primary-500" onclick={saveRegistration} disabled={!getIsOnline()}>
					Save
				</button>
				{#if !getIsOnline()}<span class="text-warning-500 text-sm">Offline</span>{/if}
			</div>
		</div>
	{/if}
</SectionCard>
