<script lang="ts">
	import { updateAgentCertificateSettings } from '$lib/api';
	import type { AgentCertificateSettings } from '$lib/types';
	import { SectionCard } from '$lib/components/ui';
	import { FormFieldRow, Checkbox, Input } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';
	import { createFormDraft } from '$lib/forms/draft.svelte';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: AgentCertificateSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	type CertDraft = {
		lifetimeDays: number;
		useAutoRenewal: boolean;
		renewalWindowHours: number;
	};

	const form = createFormDraft<CertDraft>({
		lifetimeDays: 365,
		useAutoRenewal: true,
		renewalWindowHours: 24
	});

	let saving = $state(false);

	$effect(() => {
		if (settings) {
			form.load({
				lifetimeDays: settings.lifetime_days,
				useAutoRenewal: settings.renewal_window_hours_override === null,
				renewalWindowHours: settings.renewal_window_hours_override ?? settings.effective_renewal_window_hours
			});
		}
	});

	async function saveCertificates() {
		saving = true;
		try {
			// Send 0 to reset to automatic, or the explicit value for a custom override.
			const renewalHours = form.draft.useAutoRenewal ? 0 : form.draft.renewalWindowHours;
			const res = await updateAgentCertificateSettings({
				lifetime_days: form.draft.lifetimeDays,
				renewal_window_hours: renewalHours
			});
			form.commit({
				lifetimeDays: res.lifetime_days,
				useAutoRenewal: res.renewal_window_hours_override === null,
				renewalWindowHours: res.renewal_window_hours_override ?? res.effective_renewal_window_hours
			});
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
		<p class="text-[var(--text-secondary)]">Loading...</p>
	{:else}
		<div class="space-y-4">
			<FormFieldRow
				label="Certificate Lifetime (days)"
				inputId="agent-cert-lifetime"
				dirty={form.isFieldDirty('lifetimeDays')}
			>
				<Input id="agent-cert-lifetime" type="number" min="1" max="730" bind:value={form.draft.lifetimeDays} />
			</FormFieldRow>
			<FormFieldRow
				label="Renewal Strategy"
				inputId="agent-cert-auto-renew"
				hint="Agents request a new certificate once remaining validity falls below this window."
			>
				<div class="space-y-2">
					<label class="flex items-center gap-2">
						<Checkbox id="agent-cert-auto-renew" bind:checked={form.draft.useAutoRenewal} />
						<span>Automatic renewal window</span>
					</label>
					{#if form.draft.useAutoRenewal}
						<p class="text-sm text-[var(--text-muted)]">
							Automatic: min(14 days, lifetime ÷ 5) = {settings.effective_renewal_window_hours} hours
						</p>
					{:else}
						<FormFieldRow label="Renewal Window (hours)" inputId="agent-cert-renewal-window-hours">
							<Input
								id="agent-cert-renewal-window-hours"
								type="number"
								min="1"
								bind:value={form.draft.renewalWindowHours}
							/>
						</FormFieldRow>
					{/if}
				</div>
			</FormFieldRow>
			<div class="flex gap-2">
				<Button variant="primary" loading={saving} disabled={!form.isDirty || saving} onclick={saveCertificates}
					>Save</Button
				>
				{#if form.isDirty}
					<Button variant="ghost" disabled={saving} onclick={() => form.discard()}>Discard</Button>
				{/if}
			</div>
		</div>
	{/if}
</SectionCard>
