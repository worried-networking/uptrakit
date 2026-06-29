<script lang="ts">
	import { mfaVerify, mfaSendEmail } from '$lib/api';
	import type { MfaMethod, AuthResponse } from '$lib/types';
	import { FormFieldRow, Input } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';

	let {
		mfaToken,
		availableMethods,
		onSuccess,
		onError
	}: {
		mfaToken: string;
		availableMethods: MfaMethod[];
		onSuccess: (res: AuthResponse) => void;
		onError: (msg: string) => void;
	} = $props();

	type Phase = 'totp' | 'email' | 'recovery';

	let phase = $state<Phase>('totp');
	let code = $state('');
	let loading = $state(false);
	let emailSent = $state(false);
	let errorMsg = $state('');

	const hasEmail = $derived(availableMethods.includes('email'));

	async function handleVerify(method: MfaMethod) {
		if (!code.trim()) return;
		loading = true;
		errorMsg = '';
		try {
			const { data: res } = await mfaVerify({ body: { mfa_token: mfaToken, code, method } });
			onSuccess(res as unknown as AuthResponse);
		} catch (e) {
			const msg = e instanceof Error ? e.message : 'Verification failed';
			errorMsg = msg;
			onError(msg);
			code = '';
		} finally {
			loading = false;
		}
	}

	async function sendEmail() {
		loading = true;
		errorMsg = '';
		try {
			await mfaSendEmail({ body: { mfa_token: mfaToken } });
			emailSent = true;
			code = '';
			phase = 'email';
		} catch (e) {
			const msg = e instanceof Error ? e.message : 'Failed to send email code';
			errorMsg = msg;
			onError(msg);
		} finally {
			loading = false;
		}
	}

	function switchToRecovery() {
		phase = 'recovery';
		code = '';
		errorMsg = '';
	}

	function switchToTotp() {
		phase = 'totp';
		code = '';
		errorMsg = '';
	}

	// Auto-submit when 6 digits entered for totp or email phases
	$effect(() => {
		if ((phase === 'totp' || phase === 'email') && code.length === 6 && !loading) {
			handleVerify(phase === 'email' ? 'email' : 'totp');
		}
	});
</script>

<div class="space-y-4">
	{#if errorMsg}
		<p class="text-[var(--color-danger)] text-sm">{errorMsg}</p>
	{/if}

	{#if phase === 'totp'}
		<p class="text-[var(--text-secondary)] text-sm">Enter the 6-digit code from your authenticator app.</p>
		<FormFieldRow label="Authenticator code" inputId="mfa-code">
			<Input
				id="mfa-code"
				type="text"
				inputmode="numeric"
				pattern="[0-9]*"
				maxlength={6}
				autocomplete="one-time-code"
				placeholder="000000"
				bind:value={code}
				disabled={loading}
			/>
		</FormFieldRow>
		<Button variant="primary" {loading} disabled={code.length < 6} onclick={() => handleVerify('totp')}>Verify</Button>
		<div class="flex flex-col gap-1 mt-1">
			{#if hasEmail}
				<button
					type="button"
					class="text-left text-sm text-[var(--accent)] hover:text-[var(--accent-bright)] underline underline-offset-4 disabled:opacity-40"
					disabled={loading}
					onclick={sendEmail}
				>
					Use email code instead
				</button>
			{/if}
			<button
				type="button"
				class="text-left text-sm text-[var(--accent)] hover:text-[var(--accent-bright)] underline underline-offset-4"
				onclick={switchToRecovery}
			>
				Use a recovery code
			</button>
		</div>
	{:else if phase === 'email'}
		<p class="text-[var(--text-secondary)] text-sm">
			{emailSent ? 'A 6-digit code was sent to your email address.' : 'Enter the code sent to your email.'}
		</p>
		<FormFieldRow label="Email code" inputId="mfa-email-code">
			<Input
				id="mfa-email-code"
				type="text"
				inputmode="numeric"
				pattern="[0-9]*"
				maxlength={6}
				autocomplete="one-time-code"
				placeholder="000000"
				bind:value={code}
				disabled={loading}
			/>
		</FormFieldRow>
		<Button variant="primary" {loading} disabled={code.length < 6} onclick={() => handleVerify('email')}>Verify</Button>
		<div class="flex flex-col gap-1 mt-1">
			<button
				type="button"
				class="text-left text-sm text-[var(--accent)] hover:text-[var(--accent-bright)] underline underline-offset-4 disabled:opacity-40"
				disabled={loading}
				onclick={sendEmail}
			>
				Resend code
			</button>
			<button
				type="button"
				class="text-left text-sm text-[var(--accent)] hover:text-[var(--accent-bright)] underline underline-offset-4"
				onclick={switchToTotp}
			>
				Use authenticator app instead
			</button>
		</div>
	{:else if phase === 'recovery'}
		<p class="text-[var(--text-secondary)] text-sm">
			Enter one of your recovery codes. Each code can only be used once.
		</p>
		<FormFieldRow label="Recovery code" inputId="mfa-recovery-code">
			<Input
				id="mfa-recovery-code"
				type="text"
				autocomplete="off"
				placeholder="xxxx-xxxx-xxxx"
				bind:value={code}
				disabled={loading}
			/>
		</FormFieldRow>
		<Button variant="primary" {loading} disabled={!code.trim()} onclick={() => handleVerify('recovery_code')}>
			Verify
		</Button>
		<div class="mt-1">
			<button
				type="button"
				class="text-left text-sm text-[var(--accent)] hover:text-[var(--accent-bright)] underline underline-offset-4"
				onclick={switchToTotp}
			>
				Back to authenticator app
			</button>
		</div>
	{/if}
</div>
