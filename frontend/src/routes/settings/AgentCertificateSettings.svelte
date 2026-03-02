<script lang="ts">
	import { updateAgentCertificateSettings } from '$lib/api';
	import type { AgentCertificateSettings } from '$lib/types';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: AgentCertificateSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let certLifetimeDays: number = $state(7);
	let useAutoRenewal: boolean = $state(true);
	let certRenewalWindowHours: number = $state(24);

	$effect(() => {
		if (settings) {
			certLifetimeDays = settings.lifetime_days;
			useAutoRenewal = settings.renewal_window_hours_override === null;
			certRenewalWindowHours =
				settings.renewal_window_hours_override ?? settings.effective_renewal_window_hours;
		}
	});

	async function saveCertificates() {
		try {
			// Send 0 to reset to automatic, or the explicit value for a custom override.
			const renewalHours = useAutoRenewal ? 0 : certRenewalWindowHours;
			const res = await updateAgentCertificateSettings({
				lifetime_days: certLifetimeDays,
				renewal_window_hours: renewalHours
			});
			certLifetimeDays = res.lifetime_days;
			useAutoRenewal = res.renewal_window_hours_override === null;
			certRenewalWindowHours =
				res.renewal_window_hours_override ?? res.effective_renewal_window_hours;
			onSuccess('Agent certificate settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save agent certificate settings');
		}
	}
</script>

<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">Agent Certificates</h2>
	{#if settings === undefined}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else}
		<p class="mb-4 text-surface-600 dark:text-surface-400">
			Configure the lifetime and renewal window for agent mTLS certificates. Agents will request a
			new certificate when the remaining validity falls below the renewal window.
		</p>
		<div class="mb-4 grid grid-cols-1 gap-4 sm:grid-cols-2">
			<label class="label">
				<span>Certificate Lifetime (days)</span>
				<input class="input" type="number" min="1" max="730" bind:value={certLifetimeDays} />
			</label>
			<div class="flex flex-col gap-2">
				<label class="label flex items-center gap-2">
					<input type="checkbox" class="checkbox" bind:checked={useAutoRenewal} />
					<span>Automatic renewal window</span>
				</label>
				{#if useAutoRenewal}
					<p class="text-sm text-surface-500 dark:text-surface-400">
						Automatic: min(14 days, lifetime ÷ 5) = {settings.effective_renewal_window_hours} hours
					</p>
				{:else}
					<label class="label">
						<span>Renewal Window (hours)</span>
						<input
							class="input"
							type="number"
							min="1"
							bind:value={certRenewalWindowHours}
						/>
					</label>
				{/if}
			</div>
		</div>
		<button class="btn preset-filled-primary-500" onclick={saveCertificates}> Save </button>
	{/if}
</div>
