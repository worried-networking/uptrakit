<script lang="ts">
	import { updateAgentCertificateSettings } from '$lib/api';
	import type { AgentCertificateSettings } from '$lib/types';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

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
	let saving: boolean = $state(false);

	$effect(() => {
		if (settings) {
			certLifetimeDays = settings.lifetime_days;
			useAutoRenewal = settings.renewal_window_hours_override === null;
			certRenewalWindowHours = settings.renewal_window_hours_override ?? settings.effective_renewal_window_hours;
		}
	});

	async function saveCertificates() {
		saving = true;
		try {
			// Send 0 to reset to automatic, or the explicit value for a custom override.
			const renewalHours = useAutoRenewal ? 0 : certRenewalWindowHours;
			const res = await updateAgentCertificateSettings({
				lifetime_days: certLifetimeDays,
				renewal_window_hours: renewalHours
			});
			certLifetimeDays = res.lifetime_days;
			useAutoRenewal = res.renewal_window_hours_override === null;
			certRenewalWindowHours = res.renewal_window_hours_override ?? res.effective_renewal_window_hours;
			onSuccess('Agent certificate settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save agent certificate settings');
		} finally {
			saving = false;
		}
	}
</script>

<SectionCard
	title="Agent Certificates"
	description="Configure the lifetime and renewal window for agent mTLS certificates."
>
	{#if settings === undefined}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else}
		<div class="space-y-4">
			<FormFieldRow label="Certificate Lifetime (days)" inputId="agent-cert-lifetime">
				<input id="agent-cert-lifetime" class="input" type="number" min="1" max="730" bind:value={certLifetimeDays} />
			</FormFieldRow>
			<FormFieldRow
				label="Renewal Strategy"
				inputId="agent-cert-auto-renew"
				hint="Agents request a new certificate once remaining validity falls below this window."
			>
				<div class="space-y-2">
					<label class="flex items-center gap-2">
						<input id="agent-cert-auto-renew" type="checkbox" class="checkbox" bind:checked={useAutoRenewal} />
						<span>Automatic renewal window</span>
					</label>
					{#if useAutoRenewal}
						<p class="text-sm text-surface-500 dark:text-surface-400">
							Automatic: min(14 days, lifetime ÷ 5) = {settings.effective_renewal_window_hours} hours
						</p>
					{:else}
						<label class="label">
							<span>Renewal Window (hours)</span>
							<input class="input" type="number" min="1" bind:value={certRenewalWindowHours} />
						</label>
					{/if}
				</div>
			</FormFieldRow>
			<Button variant="primary" loading={saving} onclick={saveCertificates}>Save</Button>
		</div>
	{/if}
</SectionCard>
