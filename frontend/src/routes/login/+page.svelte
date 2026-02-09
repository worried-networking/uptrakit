<script lang="ts">
	import { onMount } from 'svelte';
	import { user, handleLogin, handleOidcLogin, handleOidcCallback, handleOidcLink, handleOidcCompleteRegistration } from '$lib/auth';
	import { getAuthMethods } from '$lib/api';
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { AuthMethodsResponse } from '$lib/types';
	import { isValidLogoUrl } from '$lib/utils';

	let email = $state('');
	let password = $state('');
	let linkPassword = $state('');
	let error = $state('');
	let authMethods = $state<AuthMethodsResponse | null>(null);
	let linkRequired = $state(false);
	let linkToken = $state('');
	let linkEmail = $state('');
	let linkProviderId = $state('');
	let oidcLoading = $state(false);
	let registrationTokenRequired = $state(false);
	let registrationCode = $state('');
	let registrationTokenInput = $state('');

	$effect(() => {
		if ($user) {
			goto('/');
		}
	});

	$effect(() => {
		const params = $page.url.searchParams;

		// Handle OIDC exchange code callback
		const oidcCode = params.get('oidc_code');
		if (oidcCode) {
			handleOidcCallback(oidcCode)
				.then(() => goto(safeRedirect()))
				.catch((err) => {
					error = err instanceof Error ? err.message : 'OIDC login failed';
				});
			return;
		}

		// Handle OIDC errors
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
			error = errorMessages[oidcError] || `Authentication error: ${oidcError}`;
			return;
		}

		// Handle OIDC registration token requirement
		if (params.get('registration_token_required') === 'true') {
			registrationTokenRequired = true;
			registrationCode = params.get('registration_code') || '';
			return;
		}

		// Handle account linking
		if (params.get('link_required') === 'true') {
			linkRequired = true;
			linkToken = params.get('link_token') || '';
			linkEmail = params.get('email') || '';
			linkProviderId = params.get('link_provider_id') || '';
		}
	});

	// Load auth methods on mount
	onMount(() => {
		getAuthMethods()
			.then((methods) => {
				authMethods = methods;
			})
			.catch(() => {
				// Fallback: assume password auth
				authMethods = { password: true, oidc_providers: [], setup_required: false, registration_token_required: false };
			});
	});

	function safeRedirect(): string {
		const redirect = $page.url.searchParams.get('redirect');
		if (redirect && redirect.startsWith('/') && !redirect.startsWith('//')) {
			return redirect;
		}
		return '/';
	}

	async function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		try {
			await handleLogin({ email, password });
			goto(safeRedirect());
		} catch (err) {
			error = err instanceof Error ? err.message : 'Login failed';
		}
	}

	async function onOidcLogin(providerId: string) {
		error = '';
		oidcLoading = true;
		try {
			await handleOidcLogin(providerId);
		} catch (err) {
			error = err instanceof Error ? err.message : 'OIDC login failed';
			oidcLoading = false;
		}
	}

	async function onLinkWithPassword(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		try {
			await handleOidcLink(linkToken, linkPassword);
			goto(safeRedirect());
		} catch (err) {
			error = err instanceof Error ? err.message : 'Account linking failed';
		}
	}

	async function onSubmitRegistrationToken(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		try {
			await handleOidcCompleteRegistration(registrationCode, registrationTokenInput);
			goto(safeRedirect());
		} catch (err) {
			error = err instanceof Error ? err.message : 'Registration failed';
		}
	}

	async function onLinkWithOidc(providerId: string) {
		error = '';
		oidcLoading = true;
		try {
			// First login with the other provider, then call link with bearer token
			await handleOidcLogin(providerId);
		} catch (err) {
			error = err instanceof Error ? err.message : 'OIDC linking failed';
			oidcLoading = false;
		}
	}
</script>

<div class="card mx-auto mt-8 max-w-md p-8">
	{#if registrationTokenRequired}
		<h2 class="h2 mb-6 text-center">Registration Token Required</h2>
		<p class="mb-4 text-center text-sm text-surface-600 dark:text-surface-400">
			Enter the registration token from the controller startup logs to complete your account setup.
		</p>

		{#if error}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{error}</p>
			</aside>
		{/if}

		<form onsubmit={onSubmitRegistrationToken} class="space-y-4">
			<label class="label">
				<span>Registration Token</span>
				<input
					class="input"
					type="text"
					bind:value={registrationTokenInput}
					required
					placeholder="Paste the registration token here"
				/>
			</label>
			<button type="submit" class="btn preset-filled-primary-500 w-full">Complete Registration</button>
		</form>
	{:else if linkRequired}
		<h2 class="h2 mb-6 text-center">Link Your Account</h2>
		<p class="mb-4 text-center text-sm text-surface-600 dark:text-surface-400">
			An account with email <strong>{linkEmail}</strong> already exists. Verify your identity to link it.
		</p>
	{:else if authMethods?.setup_required}
		<h2 class="h2 mb-6 text-center">Welcome to Uptrakit</h2>
		{#if authMethods.oidc_providers.length > 0}
			<p class="mb-4 text-center text-sm text-surface-600 dark:text-surface-400">
				Sign in with your identity provider to set up your account.
			</p>
			{#if authMethods.registration_token_required}
				<p class="mb-4 text-center text-sm text-surface-600 dark:text-surface-400">
					You'll need the registration token from the controller logs after signing in.
				</p>
			{/if}
		{:else if authMethods.password}
			<p class="mb-4 text-center text-sm text-surface-600 dark:text-surface-400">
				Register your first account to get started.
			</p>
		{/if}
	{:else}
		<h2 class="h2 mb-6 text-center">Login</h2>
	{/if}

	{#if !registrationTokenRequired && error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
		</aside>
	{/if}

	{#if linkRequired}
		<!-- Account Linking UI -->
		{#if linkProviderId}
			<!-- OIDC-to-OIDC linking: offer login with the other provider -->
			<button
				type="button"
				class="btn preset-filled-secondary-500 mb-4 w-full"
				disabled={oidcLoading}
				onclick={() => onLinkWithOidc(linkProviderId)}
			>
				{oidcLoading ? 'Redirecting...' : 'Verify with linked provider'}
			</button>
			<div class="my-4 flex items-center gap-4">
				<hr class="flex-1" />
				<span class="text-sm text-surface-600 dark:text-surface-400">or</span>
				<hr class="flex-1" />
			</div>
		{/if}

		<!-- Password linking form -->
		<form onsubmit={onLinkWithPassword} class="space-y-4">
			<label class="label">
				<span>Password</span>
				<input
					class="input"
					type="password"
					bind:value={linkPassword}
					required
					autocomplete="current-password"
					placeholder="Enter your password to verify"
				/>
			</label>
			<button type="submit" class="btn preset-filled-primary-500 w-full">Link Account</button>
		</form>
	{:else if !registrationTokenRequired}
		<!-- Normal Login UI -->
		{#if authMethods}
			<!-- OIDC Provider Buttons -->
			{#each authMethods.oidc_providers as provider}
				<button
					type="button"
					class="btn preset-filled-secondary-500 mb-3 flex w-full items-center justify-center gap-2"
					disabled={oidcLoading}
					onclick={() => onOidcLogin(provider.id)}
				>
					{#if isValidLogoUrl(provider.logo_url)}
						<img src={provider.logo_url} alt="" class="h-5 w-5" referrerpolicy="no-referrer" />
					{/if}
					{oidcLoading ? 'Redirecting...' : `Login with ${provider.name}`}
				</button>
			{/each}

			<!-- Divider if both methods available -->
			{#if authMethods.oidc_providers.length > 0 && authMethods.password}
				<div class="my-4 flex items-center gap-4">
					<hr class="flex-1" />
					<span class="text-sm text-surface-600 dark:text-surface-400">or</span>
					<hr class="flex-1" />
				</div>
			{/if}

			<!-- Password Login Form -->
			{#if authMethods.password}
				<form onsubmit={onSubmit} class="space-y-4">
					<label class="label">
						<span>Email</span>
						<input
							class="input"
							type="email"
							bind:value={email}
							required
							autocomplete="email"
						/>
					</label>

					<label class="label">
						<span>Password</span>
						<input
							class="input"
							type="password"
							bind:value={password}
							required
							autocomplete="current-password"
						/>
					</label>

					<button type="submit" class="btn preset-filled-primary-500 w-full">Login</button>
				</form>

				<p class="mt-4 text-center">
					Don't have an account? <a href="/register" class="anchor">Register</a>
				</p>
			{/if}
		{:else}
			<p class="text-center text-surface-600 dark:text-surface-400">Loading...</p>
		{/if}
	{/if}
</div>
