<script lang="ts">
	import { goto } from '$app/navigation';
	import { getUser, handleRegister } from '$lib/auth.svelte';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { Callout } from '$lib/components/ui';
	import { FormFieldRow, Input, Checkbox } from '$lib/components/forms';
	import PublicEntryShell, { PUBLIC_ENTRY_FORM_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
	import Button from '$lib/components/Button.svelte';
	import Link from '$lib/components/Link.svelte';

	type RegisterFieldErrors = {
		email: string;
		firstName: string;
		lastName: string;
		password: string;
	};

	let email = $state('');
	let firstName = $state('');
	let lastName = $state('');
	let password = $state('');
	let showToken = $state(false);
	let registrationToken = $state('');
	let error = $state('');
	let hasRedirected = false;
	let registerFieldErrors = $state<RegisterFieldErrors>({
		email: '',
		firstName: '',
		lastName: '',
		password: ''
	});

	$effect(() => {
		if (getUser() && !hasRedirected) {
			hasRedirected = true;
			goto('/');
		}
	});

	function clearRegisterFieldError(field: keyof RegisterFieldErrors) {
		registerFieldErrors[field] = '';
	}

	function validateRegisterFields() {
		registerFieldErrors = {
			email: email.trim() ? '' : 'Email is required.',
			firstName: firstName.trim() ? '' : 'First name is required.',
			lastName: lastName.trim() ? '' : 'Last name is required.',
			password: password.trim()
				? password.trim().length >= 8
					? ''
					: 'Password must be at least 8 characters.'
				: 'Password is required.'
		};
		return (
			!registerFieldErrors.email &&
			!registerFieldErrors.firstName &&
			!registerFieldErrors.lastName &&
			!registerFieldErrors.password
		);
	}

	async function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		if (!validateRegisterFields()) return;
		try {
			await handleRegister({
				email,
				first_name: firstName,
				last_name: lastName,
				password,
				...(showToken && registrationToken ? { registration_token: registrationToken } : {})
			});
			goto('/');
		} catch (err) {
			error = err instanceof Error ? err.message : 'Registration failed';
		}
	}
</script>

<PublicEntryShell eyebrow="Authentication" title="Register" subtitle="Create your local account to sign in later.">
	<Callout
		tone="info"
		message="Fields marked with an asterisk are required. Invite tokens are optional unless your deployment requires one."
	/>

	{#if error}
		<Callout tone="danger" title="Registration failed" message={error} />
	{/if}

	<form class={PUBLIC_ENTRY_FORM_CLASS} novalidate onsubmit={onSubmit}>
		<FormFieldRow label="Email" inputId="register-email" required error={registerFieldErrors.email || undefined}>
			<Input
				id="register-email"
				type="email"
				bind:value={email}
				autocomplete="email"
				error={registerFieldErrors.email || undefined}
				oninput={() => clearRegisterFieldError('email')}
			/>
		</FormFieldRow>

		<FormFieldRow
			label="First name"
			inputId="register-first-name"
			required
			error={registerFieldErrors.firstName || undefined}
		>
			<Input
				id="register-first-name"
				type="text"
				bind:value={firstName}
				autocomplete="given-name"
				error={registerFieldErrors.firstName || undefined}
				oninput={() => clearRegisterFieldError('firstName')}
			/>
		</FormFieldRow>

		<FormFieldRow
			label="Last name"
			inputId="register-last-name"
			required
			error={registerFieldErrors.lastName || undefined}
		>
			<Input
				id="register-last-name"
				type="text"
				bind:value={lastName}
				autocomplete="family-name"
				error={registerFieldErrors.lastName || undefined}
				oninput={() => clearRegisterFieldError('lastName')}
			/>
		</FormFieldRow>

		<FormFieldRow
			label="Password"
			inputId="register-password"
			required
			hint="Use at least 8 characters."
			error={registerFieldErrors.password || undefined}
		>
			<Input
				id="register-password"
				type="password"
				bind:value={password}
				autocomplete="new-password"
				error={registerFieldErrors.password || undefined}
				oninput={() => clearRegisterFieldError('password')}
			/>
		</FormFieldRow>

		<label class="flex items-start gap-2 text-sm text-[var(--text-secondary)]">
			<Checkbox
				id="show-token"
				checked={showToken}
				onchange={(e) => {
					showToken = (e.target as HTMLInputElement).checked;
					if (!showToken) registrationToken = '';
				}}
			/>
			<span>I have an invite token</span>
		</label>

		{#if showToken}
			<FormFieldRow
				label="Invite token"
				inputId="register-token"
				hint="Optional unless your organization provided one."
			>
				<Input id="register-token" type="text" bind:value={registrationToken} autocomplete="off" />
			</FormFieldRow>
		{/if}

		<div class="space-y-2">
			<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>Register</Button>
			{#if !getIsOnline()}
				<p class="text-sm text-[var(--text-muted)]">Offline</p>
			{/if}
		</div>
	</form>

	{#snippet footer()}
		<p class="text-center text-sm text-[var(--text-secondary)]">
			Already have an account? <Link href="/login">Login</Link>
		</p>
	{/snippet}
</PublicEntryShell>
