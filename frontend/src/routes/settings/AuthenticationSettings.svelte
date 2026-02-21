<script lang="ts">
	import { updateAuthenticationSettings } from '$lib/api';
	import type { AuthenticationSettings } from '$lib/types';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let passwordAuthEnabled: boolean = $state(true);

	export async function load(settings: AuthenticationSettings) {
		passwordAuthEnabled = settings.password_auth_enabled;
	}

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
	<label class="mb-4 flex items-center gap-3">
		<input class="checkbox" type="checkbox" bind:checked={passwordAuthEnabled} />
		<span>Enable password authentication</span>
	</label>
	<button class="btn preset-filled-primary-500" onclick={saveAuthentication}> Save </button>
</div>
