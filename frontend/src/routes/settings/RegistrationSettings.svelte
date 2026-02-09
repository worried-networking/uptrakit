<script lang="ts">
	import { updateRegistrationSettings } from '$lib/api';
	import type { RegistrationSettings } from '$lib/types';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let regMode: 'open' | 'invite' | 'closed' = $state('open');
	let regToken: string = $state('');
	let regRequireTokenForOidc: boolean = $state(false);

	export async function load(settings: RegistrationSettings) {
		regMode = settings.mode;
		regRequireTokenForOidc = settings.require_token_for_oidc;
	}

	async function saveRegistration() {
		try {
			const data: { mode: 'open' | 'invite' | 'closed'; token?: string; require_token_for_oidc?: boolean } = { mode: regMode };
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

<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">Registration</h2>
	<label class="label mb-4">
		<span>Registration Mode</span>
		<select class="select" bind:value={regMode}>
			<option value="open">Open</option>
			<option value="invite">Invite Only</option>
			<option value="closed">Closed</option>
		</select>
	</label>

	{#if regMode === 'invite'}
		<label class="label mb-4">
			<span>Registration Token</span>
			<input
				class="input"
				type="text"
				placeholder="Enter a new registration token"
				bind:value={regToken}
			/>
			<small class="text-surface-600 dark:text-surface-400">Set a new token for invite-only registration. Leave blank to keep the current token.</small>
		</label>

		<label class="mb-4 flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={regRequireTokenForOidc} />
			<span>Require registration token for OIDC users</span>
		</label>
		<small class="mb-4 block text-surface-600 dark:text-surface-400">When enabled, users signing in via OIDC for the first time must also provide the registration token.</small>
	{/if}

	<button class="btn preset-filled-primary-500" onclick={saveRegistration}>
		Save
	</button>
</div>
