<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import { getAuthMethods } from '$lib/api';
	import {
		getUser,
		handleLogin,
		handleOidcCallback,
		handleOidcCompleteRegistration,
		handleOidcLink,
		handleOidcLogin
	} from '$lib/auth.svelte';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import type { AuthMethodsResponse } from '$lib/types';
	import { isValidLogoUrl, safeRedirect as safeRedirectFn } from '$lib/utils';
	import { Callout, FormFieldRow } from '$lib/components/ui';
	import PublicEntryShell, {
		PUBLIC_ENTRY_FORM_CLASS,
		PUBLIC_ENTRY_INPUT_CLASS,
		PUBLIC_ENTRY_LINK_CLASS
	} from '$lib/components/ui/PublicEntryShell.svelte';
	import Button from '$lib/components/Button.svelte';

	type LoginFieldErrors = {
		email: string;
		password: string;
	};

	let email = $state('');
	let password = $state('');
	let linkPassword = $state('');
	let authMethods = $state<AuthMethodsResponse | null>(null);
	let linkRequired = $state(false);
	let linkToken = $state('');
	let linkEmail = $state('');
	let linkProviderId = $state('');
	let oidcLoading = $state(false);
	let registrationTokenRequired = $state(false);
	let registrationCode = $state('');
	let registrationTokenInput = $state('');
	let hasRedirected = false;
	let processedOidcCode: string | null = null;
	let bannerError = $state('');
	let loginFieldErrors = $state<LoginFieldErrors>({ email: '', password: '' });
	let registrationTokenError = $state('');
	let linkPasswordError = $state('');

	$effect(() => {
		if (getUser() && !hasRedirected) {
			hasRedirected = true;
			goto('/');
		}
	});

	$effect(() => {
		const params = page.url.searchParams;

		const oidcCode = params.get('oidc_code');
		if (oidcCode && processedOidcCode !== oidcCode) {
			processedOidcCode = oidcCode;
			handleOidcCallback(oidcCode)
				.then(() => goto(safeRedirect()))
				.catch((err) => {
					bannerError = err instanceof Error ? err.message : 'OIDC login failed';
				});
			return;
		}

		const oidcError = params.get('error');
		if (oidcError) {
			const errorMessages: Record<string, string> = {
				oidc_denied: 'Authentication was denied by the provider',
				oidc_no_account: 'No account found and automatic creation is disabled',
				account_deactivated: 'Your account has been deactivated',
				oidc_state_expired: 'Authentication session expired, please try again',
				oidc_no_email: 'Email address is required from the provider',
				oidc_discovery_failed: 'Failed to connect to the authentication provider',
				oidc_token_exchange_failed: 'Authentication failed during token exchange',
				oidc_token_validation_failed: 'Failed to validate authentication token'
			};
			bannerError = errorMessages[oidcError] || 'Authentication failed. Please try again.';
			return;
		}

		const hash = page.url.hash.slice(1);
		const hashParams = new URLSearchParams(hash);
		if (hashParams.get('registration_token_required') === 'true') {
			registrationTokenRequired = true;
			registrationCode = hashParams.get('registration_code') || '';
			return;
		}

		if (params.get('link_required') === 'true') {
			linkRequired = true;
			linkToken = hashParams.get('link_token') || params.get('link_token') || '';
			linkEmail = params.get('email') || '';
			linkProviderId = params.get('link_provider_id') || '';
		}
	});

	onMount(() => {
		getAuthMethods()
			.then((methods) => {
				authMethods = methods;
			})
			.catch(() => {
				authMethods = {
					password: true,
					oidc_providers: [],
					setup_required: false,
					registration_token_required: false
				};
			});
	});

	function safeRedirect(): string {
		return safeRedirectFn(page.url.searchParams.get('redirect'));
	}

	function loginTitle(): string {
		if (registrationTokenRequired) return 'Complete registration';
		if (linkRequired) return 'Link your account';
		if (authMethods?.setup_required) return 'Welcome to Uptrakit';
		return 'Login';
	}

	function loginSubtitle(): string {
		if (registrationTokenRequired) {
			return 'Enter the registration token from the controller logs to finish account setup.';
		}
		if (linkRequired) {
			return `Verify your identity so we can connect the existing account for ${linkEmail}.`;
		}
		if (authMethods?.setup_required) {
			return authMethods.oidc_providers.length > 0
				? 'Sign in with an identity provider to create the first account.'
				: 'Register the first account with email and password.';
		}
		return 'Use your account credentials or an identity provider.';
	}

	function clearLoginFieldError(field: keyof LoginFieldErrors) {
		loginFieldErrors[field] = '';
	}

	function validateLoginFields() {
		loginFieldErrors = {
			email: email.trim() ? '' : 'Email is required.',
			password: password.trim() ? '' : 'Password is required.'
		};
		return !loginFieldErrors.email && !loginFieldErrors.password;
	}

	function clearRegistrationTokenError() {
		registrationTokenError = '';
	}

	function validateRegistrationToken() {
		registrationTokenError = registrationTokenInput.trim() ? '' : 'Registration token is required.';
		return !registrationTokenError;
	}

	function clearLinkPasswordError() {
		linkPasswordError = '';
	}

	function validateLinkPassword() {
		linkPasswordError = linkPassword.trim() ? '' : 'Password is required.';
		return !linkPasswordError;
	}

	async function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		bannerError = '';
		if (!validateLoginFields()) return;
		try {
			await handleLogin({ email, password });
			goto(safeRedirect());
		} catch (err) {
			bannerError = err instanceof Error ? err.message : 'Login failed';
		}
	}

	async function onOidcLogin(providerId: string) {
		bannerError = '';
		oidcLoading = true;
		try {
			await handleOidcLogin(providerId);
		} catch (err) {
			bannerError = err instanceof Error ? err.message : 'OIDC login failed';
			oidcLoading = false;
		}
	}

	async function onLinkWithPassword(e: SubmitEvent) {
		e.preventDefault();
		bannerError = '';
		if (!validateLinkPassword()) return;
		try {
			await handleOidcLink(linkToken, linkPassword);
			goto(safeRedirect());
		} catch (err) {
			bannerError = err instanceof Error ? err.message : 'Account linking failed';
		}
	}

	async function onSubmitRegistrationToken(e: SubmitEvent) {
		e.preventDefault();
		bannerError = '';
		if (!validateRegistrationToken()) return;
		try {
			await handleOidcCompleteRegistration(registrationCode, registrationTokenInput);
			goto(safeRedirect());
		} catch (err) {
			bannerError = err instanceof Error ? err.message : 'Registration failed';
		}
	}

	async function onLinkWithOidc(providerId: string) {
		bannerError = '';
		oidcLoading = true;
		try {
			await handleOidcLogin(providerId);
		} catch (err) {
			bannerError = err instanceof Error ? err.message : 'OIDC linking failed';
			oidcLoading = false;
		}
	}
</script>

<PublicEntryShell eyebrow="Authentication" title={loginTitle()} subtitle={loginSubtitle()}>
	{#if bannerError}
		<Callout tone="danger" title="Authentication failed" message={bannerError} />
	{/if}

	{#if registrationTokenRequired}
		<Callout
			tone="info"
			message="Use the registration token from the controller logs to finish the first account setup."
		/>

		<form class={PUBLIC_ENTRY_FORM_CLASS} novalidate onsubmit={onSubmitRegistrationToken}>
			<FormFieldRow
				label="Registration token"
				inputId="registration-token"
				required
				error={registrationTokenError || undefined}
			>
				<input
					id="registration-token"
					class={PUBLIC_ENTRY_INPUT_CLASS}
					type="text"
					bind:value={registrationTokenInput}
					placeholder="Paste the registration token here"
					aria-invalid={registrationTokenError ? 'true' : undefined}
					oninput={clearRegistrationTokenError}
				/>
			</FormFieldRow>

			<div class="space-y-2">
				<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>
					Complete registration
				</Button>
				{#if !getIsOnline()}
					<p class="text-sm text-[var(--text-muted)]">Offline</p>
				{/if}
			</div>
		</form>
	{:else if linkRequired}
		<Callout
			tone="warning"
			title="Link your account"
			message={`An account with email ${linkEmail} already exists. Verify your identity to link it.`}
		/>

		{#if linkProviderId}
			<Button
				variant="ghost"
				type="button"
				class="w-full justify-center"
				disabled={oidcLoading}
				loading={oidcLoading}
				onclick={() => onLinkWithOidc(linkProviderId)}
			>
				Verify with linked provider
			</Button>
			<div class="my-4 flex items-center gap-4" aria-hidden="true">
				<hr class="flex-1 border-[var(--border-subtle)]" />
				<span class="text-sm text-[var(--text-muted)]">or</span>
				<hr class="flex-1 border-[var(--border-subtle)]" />
			</div>
		{/if}

		<form class={PUBLIC_ENTRY_FORM_CLASS} novalidate onsubmit={onLinkWithPassword}>
			<FormFieldRow label="Password" inputId="link-password" required error={linkPasswordError || undefined}>
				<input
					id="link-password"
					class={PUBLIC_ENTRY_INPUT_CLASS}
					type="password"
					bind:value={linkPassword}
					autocomplete="current-password"
					placeholder="Enter your password to verify"
					aria-invalid={linkPasswordError ? 'true' : undefined}
					oninput={clearLinkPasswordError}
				/>
			</FormFieldRow>

			<div class="space-y-2">
				<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>
					Link account
				</Button>
				{#if !getIsOnline()}
					<p class="text-sm text-[var(--text-muted)]">Offline</p>
				{/if}
			</div>
		</form>
	{:else}
		{#if authMethods?.setup_required}
			<Callout
				tone="info"
				message={authMethods.oidc_providers.length > 0
					? 'Use an identity provider to create the first account.'
					: 'Password authentication is enabled for first-account setup.'}
			/>
		{/if}

		{#if authMethods?.oidc_providers.length}
			<div class="space-y-3">
				{#each authMethods.oidc_providers as provider (provider.id)}
					{#snippet providerLogo()}
						{#if isValidLogoUrl(provider.logo_url)}
							<img src={provider.logo_url} alt="" class="h-5 w-5" referrerpolicy="no-referrer" />
						{/if}
					{/snippet}

					<Button
						variant="ghost"
						type="button"
						class="w-full justify-center"
						disabled={oidcLoading}
						loading={oidcLoading}
						leadingIcon={providerLogo}
						onclick={() => onOidcLogin(provider.id)}
					>
						Login with {provider.name}
					</Button>
				{/each}
			</div>
		{/if}

		{#if authMethods?.oidc_providers.length && authMethods.password}
			<div class="my-4 flex items-center gap-4" aria-hidden="true">
				<hr class="flex-1 border-[var(--border-subtle)]" />
				<span class="text-sm text-[var(--text-muted)]">or</span>
				<hr class="flex-1 border-[var(--border-subtle)]" />
			</div>
		{/if}

		{#if authMethods?.password}
			<form class={PUBLIC_ENTRY_FORM_CLASS} novalidate onsubmit={onSubmit}>
				<FormFieldRow label="Email" inputId="login-email" required error={loginFieldErrors.email || undefined}>
					<input
						id="login-email"
						class={PUBLIC_ENTRY_INPUT_CLASS}
						type="email"
						bind:value={email}
						autocomplete="email"
						aria-invalid={loginFieldErrors.email ? 'true' : undefined}
						oninput={() => clearLoginFieldError('email')}
					/>
				</FormFieldRow>

				<FormFieldRow label="Password" inputId="login-password" required error={loginFieldErrors.password || undefined}>
					<input
						id="login-password"
						class={PUBLIC_ENTRY_INPUT_CLASS}
						type="password"
						bind:value={password}
						autocomplete="current-password"
						aria-invalid={loginFieldErrors.password ? 'true' : undefined}
						oninput={() => clearLoginFieldError('password')}
					/>
				</FormFieldRow>

				<div class="space-y-2">
					<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>Login</Button>
					{#if !getIsOnline()}
						<p class="text-sm text-[var(--text-muted)]">Offline</p>
					{/if}
				</div>
			</form>
		{:else if !authMethods}
			<Callout tone="info" message="Loading sign-in options..." />
		{/if}
	{/if}

	{#snippet footer()}
		<p class="text-center text-sm text-[var(--text-secondary)]">
			Don't have an account? <a href="/register" class={PUBLIC_ENTRY_LINK_CLASS}>Register</a>
		</p>
	{/snippet}
</PublicEntryShell>
