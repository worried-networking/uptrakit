<script lang="ts">
	import { user, handleLogin, handleOidcLogin, handleOidcCallback, handleOidcLink } from '$lib/auth';
	import { getAuthMethods } from '$lib/api';
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { AuthMethodsResponse } from '$lib/types';

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
				.then(() => goto('/'))
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

		// Handle account linking
		if (params.get('link_required') === 'true') {
			linkRequired = true;
			linkToken = params.get('link_token') || '';
			linkEmail = params.get('email') || '';
			linkProviderId = params.get('link_provider_id') || '';
		}
	});

	// Load auth methods on mount
	$effect(() => {
		getAuthMethods()
			.then((methods) => {
				authMethods = methods;
			})
			.catch(() => {
				// Fallback: assume password auth
				authMethods = { password: true, oidc_providers: [] };
			});
	});

	async function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		try {
			await handleLogin({ email, password });
			goto('/');
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
			goto('/');
		} catch (err) {
			error = err instanceof Error ? err.message : 'Account linking failed';
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
	{#if linkRequired}
		<h2 class="h2 mb-6 text-center">Link Your Account</h2>
		<p class="mb-4 text-center text-sm text-surface-600-300-token">
			An account with email <strong>{linkEmail}</strong> already exists. Verify your identity to link it.
		</p>
	{:else}
		<h2 class="h2 mb-6 text-center">Login</h2>
	{/if}

	{#if error}
		<aside class="alert variant-filled-error mb-4">
			<div class="alert-message">
				<p>{error}</p>
			</div>
		</aside>
	{/if}

	{#if linkRequired}
		<!-- Account Linking UI -->
		{#if linkProviderId}
			<!-- OIDC-to-OIDC linking: offer login with the other provider -->
			<button
				type="button"
				class="btn variant-filled-secondary w-full mb-4"
				disabled={oidcLoading}
				onclick={() => onLinkWithOidc(linkProviderId)}
			>
				{oidcLoading ? 'Redirecting...' : 'Verify with linked provider'}
			</button>
			<div class="my-4 flex items-center gap-4">
				<hr class="flex-1" />
				<span class="text-sm text-surface-600-300-token">or</span>
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
			<button type="submit" class="btn variant-filled-primary w-full">Link Account</button>
		</form>
	{:else}
		<!-- Normal Login UI -->
		{#if authMethods}
			<!-- OIDC Provider Buttons -->
			{#each authMethods.oidc_providers as provider}
				<button
					type="button"
					class="btn variant-filled-secondary w-full mb-3 flex items-center justify-center gap-2"
					disabled={oidcLoading}
					onclick={() => onOidcLogin(provider.id)}
				>
					{#if provider.logo_url}
						<img src={provider.logo_url} alt="" class="h-5 w-5" />
					{/if}
					{oidcLoading ? 'Redirecting...' : `Login with ${provider.name}`}
				</button>
			{/each}

			<!-- Divider if both methods available -->
			{#if authMethods.oidc_providers.length > 0 && authMethods.password}
				<div class="my-4 flex items-center gap-4">
					<hr class="flex-1" />
					<span class="text-sm text-surface-600-300-token">or</span>
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

					<button type="submit" class="btn variant-filled-primary w-full">Login</button>
				</form>

				<p class="mt-4 text-center">
					Don't have an account? <a href="/register" class="anchor">Register</a>
				</p>
			{/if}
		{:else}
			<p class="text-center text-surface-600-300-token">Loading...</p>
		{/if}
	{/if}
</div>
