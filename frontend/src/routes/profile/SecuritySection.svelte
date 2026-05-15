<script lang="ts">
	import { mfaStatus, mfaEnroll, mfaConfirm, mfaDisable, mfaRegenerateCodes } from '$lib/api';
	import type { MfaStatusResponse, TotpEnrollResponse } from '$lib/types';
	import { SectionCard, Callout } from '$lib/components/ui';
	import { FormFieldRow, Input } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';

	let status = $state<MfaStatusResponse | null>(null);
	let enrollData = $state<TotpEnrollResponse | null>(null);
	let confirmCode = $state('');
	let recoveryCodes = $state<string[]>([]);
	let disablePassword = $state('');
	let disableTotpCode = $state('');
	let regenPassword = $state('');
	let regenTotpCode = $state('');
	let newRecoveryCodes = $state<string[]>([]);
	let loading = $state(false);
	let errorMsg = $state('');
	let showDisableForm = $state(false);
	let showRegenForm = $state(false);

	type Phase = 'idle' | 'confirming' | 'codes_shown';
	let phase = $state<Phase>('idle');

	async function loadStatus() {
		try {
			status = await mfaStatus();
		} catch {
			// Silently degrade — 2FA status is non-critical
		}
	}

	$effect(() => {
		loadStatus().catch(console.error);
	});

	async function startEnroll() {
		loading = true;
		errorMsg = '';
		try {
			enrollData = await mfaEnroll();
			phase = 'confirming';
		} catch (e) {
			errorMsg = e instanceof Error ? e.message : 'Enrollment failed';
		} finally {
			loading = false;
		}
	}

	async function confirmEnroll() {
		if (!confirmCode || confirmCode.length !== 6) return;
		loading = true;
		errorMsg = '';
		try {
			const res = await mfaConfirm({ code: confirmCode });
			recoveryCodes = res.recovery_codes;
			phase = 'codes_shown';
			await loadStatus();
		} catch (e) {
			errorMsg = e instanceof Error ? e.message : 'Invalid code';
		} finally {
			loading = false;
		}
	}

	async function disable() {
		if (!disablePassword && !disableTotpCode) {
			errorMsg = 'Enter your password or authenticator code to confirm.';
			return;
		}
		loading = true;
		errorMsg = '';
		try {
			await mfaDisable(disablePassword ? { password: disablePassword } : { totp_code: disableTotpCode });
			showDisableForm = false;
			disablePassword = '';
			disableTotpCode = '';
			await loadStatus();
		} catch (e) {
			errorMsg = e instanceof Error ? e.message : 'Failed to disable 2FA';
		} finally {
			loading = false;
		}
	}

	async function regenerate() {
		if (!regenPassword && !regenTotpCode) {
			errorMsg = 'Enter your password or authenticator code to confirm.';
			return;
		}
		loading = true;
		errorMsg = '';
		try {
			const res = await mfaRegenerateCodes(regenPassword ? { password: regenPassword } : { totp_code: regenTotpCode });
			newRecoveryCodes = res.recovery_codes;
			showRegenForm = false;
			regenPassword = '';
			regenTotpCode = '';
			await loadStatus();
		} catch (e) {
			errorMsg = e instanceof Error ? e.message : 'Failed to regenerate codes';
		} finally {
			loading = false;
		}
	}
</script>

<SectionCard title="Two-Factor Authentication">
	{#if errorMsg}
		<p class="text-[var(--color-danger)] text-sm mb-3">{errorMsg}</p>
	{/if}

	{#if status === null}
		<p class="text-[var(--text-secondary)] text-sm">Loading…</p>
	{:else if phase === 'idle'}
		{#if status.totp_enrolled}
			<p class="text-sm mb-3">
				Authenticator app is active.
				<strong>{status.recovery_codes_count}</strong>
				{status.recovery_codes_count === 1 ? 'recovery code' : 'recovery codes'} remaining.
			</p>
			<div class="flex gap-2 flex-wrap mb-4">
				<Button
					variant="danger"
					onclick={() => {
						showDisableForm = !showDisableForm;
						showRegenForm = false;
						errorMsg = '';
					}}
				>
					Disable 2FA
				</Button>
				<Button
					variant="secondary"
					onclick={() => {
						showRegenForm = !showRegenForm;
						showDisableForm = false;
						errorMsg = '';
					}}
				>
					Regenerate recovery codes
				</Button>
			</div>

			{#if showDisableForm}
				<div class="mt-2 space-y-3 border border-[var(--border)] rounded-card p-4">
					<p class="text-sm text-[var(--text-secondary)]">Confirm with your password or authenticator code:</p>
					<FormFieldRow label="Password" inputId="disable-password">
						<Input id="disable-password" type="password" bind:value={disablePassword} disabled={loading} />
					</FormFieldRow>
					<p class="text-xs text-[var(--text-secondary)]">— or —</p>
					<FormFieldRow label="Authenticator code" inputId="disable-totp">
						<Input
							id="disable-totp"
							type="text"
							inputmode="numeric"
							maxlength={6}
							bind:value={disableTotpCode}
							disabled={loading}
						/>
					</FormFieldRow>
					<Button variant="danger" {loading} onclick={disable}>Confirm disable</Button>
				</div>
			{/if}

			{#if showRegenForm}
				<div class="mt-2 space-y-3 border border-[var(--border)] rounded-card p-4">
					<p class="text-sm text-[var(--text-secondary)]">
						Confirm regeneration with your password or authenticator code:
					</p>
					<FormFieldRow label="Password" inputId="regen-password">
						<Input id="regen-password" type="password" bind:value={regenPassword} disabled={loading} />
					</FormFieldRow>
					<p class="text-xs text-[var(--text-secondary)]">— or —</p>
					<FormFieldRow label="Authenticator code" inputId="regen-totp">
						<Input
							id="regen-totp"
							type="text"
							inputmode="numeric"
							maxlength={6}
							bind:value={regenTotpCode}
							disabled={loading}
						/>
					</FormFieldRow>
					<Button variant="primary" {loading} onclick={regenerate}>Generate new codes</Button>
				</div>
			{/if}

			{#if newRecoveryCodes.length > 0}
				<div class="mt-4">
					<Callout
						tone="warning"
						title="New recovery codes — save these now"
						message="These codes will not be shown again. Store them somewhere safe."
					>
						<ul class="mt-2 font-mono text-sm space-y-1">
							{#each newRecoveryCodes as c (c)}
								<li>{c}</li>
							{/each}
						</ul>
					</Callout>
				</div>
			{/if}
		{:else}
			<p class="text-sm text-[var(--text-secondary)] mb-3">
				No authenticator app is set up. Add 2FA to protect your account.
			</p>
			<Button variant="primary" {loading} onclick={startEnroll}>Set up authenticator app</Button>
		{/if}
	{:else if phase === 'confirming' && enrollData}
		<div class="space-y-4">
			<p class="text-sm text-[var(--text-secondary)]">
				Scan the QR code below with your authenticator app (Google Authenticator, Authy, 1Password, etc.), or enter the
				secret manually.
			</p>
			<div class="p-3 bg-[var(--bg-raised)] rounded-card border border-[var(--border-subtle)]">
				<p class="text-xs font-mono break-all">{enrollData.secret}</p>
				<p class="text-xs text-[var(--text-secondary)] mt-1">Manual entry secret (Base32)</p>
			</div>
			<p class="text-sm">Then enter the 6-digit code from your app to confirm:</p>
			<FormFieldRow label="Code from app" inputId="confirm-code">
				<Input
					id="confirm-code"
					type="text"
					inputmode="numeric"
					pattern="[0-9]*"
					maxlength={6}
					autocomplete="one-time-code"
					placeholder="000000"
					bind:value={confirmCode}
					disabled={loading}
				/>
			</FormFieldRow>
			<div class="flex gap-2">
				<Button variant="primary" {loading} disabled={confirmCode.length < 6} onclick={confirmEnroll}>Confirm</Button>
				<Button
					variant="ghost"
					onclick={() => {
						phase = 'idle';
						enrollData = null;
						confirmCode = '';
						errorMsg = '';
					}}
				>
					Cancel
				</Button>
			</div>
		</div>
	{:else if phase === 'codes_shown'}
		<div class="space-y-3">
			<Callout
				tone="success"
				title="2FA is now enabled!"
				message="Save these recovery codes somewhere safe. Each can only be used once, and you will not see them again."
			>
				<ul class="mt-2 font-mono text-sm space-y-1">
					{#each recoveryCodes as c (c)}
						<li>{c}</li>
					{/each}
				</ul>
			</Callout>
			<Button
				variant="primary"
				onclick={() => {
					phase = 'idle';
					recoveryCodes = [];
				}}
			>
				Done
			</Button>
		</div>
	{/if}
</SectionCard>
