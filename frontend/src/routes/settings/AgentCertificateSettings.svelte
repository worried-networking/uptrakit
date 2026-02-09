<script lang="ts">
	import { updateAgentCertificateSettings } from '$lib/api';
	import type { AgentCertificateSettings } from '$lib/types';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let certLifetimeDays: number = $state(7);
	let certRenewalWindowHours: number = $state(6);

	export function load(settings: AgentCertificateSettings) {
		certLifetimeDays = settings.lifetime_days;
		certRenewalWindowHours = settings.renewal_window_hours;
	}

	async function saveCertificates() {
		try {
			const res = await updateAgentCertificateSettings({
				lifetime_days: certLifetimeDays,
				renewal_window_hours: certRenewalWindowHours
			});
			certLifetimeDays = res.lifetime_days;
			certRenewalWindowHours = res.renewal_window_hours;
			onSuccess('Agent certificate settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save agent certificate settings');
		}
	}
</script>

<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">Agent Certificates</h2>
	<p class="mb-4 text-surface-600 dark:text-surface-400">
		Configure the lifetime and renewal window for agent mTLS certificates.
		Agents will request a new certificate when the remaining validity falls below the renewal window.
	</p>
	<div class="mb-4 grid grid-cols-1 gap-4 sm:grid-cols-2">
		<label class="label">
			<span>Certificate Lifetime (days)</span>
			<input
				class="input"
				type="number"
				min="1"
				bind:value={certLifetimeDays}
			/>
		</label>
		<label class="label">
			<span>Renewal Window (hours)</span>
			<input
				class="input"
				type="number"
				min="1"
				bind:value={certRenewalWindowHours}
			/>
		</label>
	</div>
	<button class="btn preset-filled-primary-500" onclick={saveCertificates}>
		Save
	</button>
</div>
