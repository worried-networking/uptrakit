<script lang="ts">
	import { user } from '$lib/auth';
	import { goto } from '$app/navigation';
	import {
		getRegistrationSettings,
		updateRegistrationSettings,
		getAuthenticationSettings,
		updateAuthenticationSettings,
		getAgentCertificateSettings,
		updateAgentCertificateSettings,
		getEnrollmentTokenStatus,
		createEnrollmentToken,
		revokeEnrollmentToken,
		getOidcProviders,
		createOidcProvider,
		updateOidcProvider,
		deleteOidcProvider,
		activateOidcProvider,
		deactivateOidcProvider,
		getMqttClient,
		createMqttClient,
		updateMqttClient,
		deleteMqttClient
	} from '$lib/api';
	import {
		Permission,
		type RegistrationSettings,
		type AuthenticationSettings,
		type AgentCertificateSettings,
		type EnrollmentTokenStatus,
		type OidcProviderResponse,
		type CreateOidcProviderRequest,
		type UpdateOidcProviderRequest,
		type MqttClientResponse
	} from '$lib/types';

	// --- Global feedback ---
	let successMessage: string | null = $state(null);
	let errorMessage: string | null = $state(null);

	// --- Section data ---
	let regMode: 'open' | 'invite' | 'closed' = $state('open');
	let regToken: string = $state('');
	let regRequireTokenForOidc: boolean = $state(false);

	let passwordAuthEnabled: boolean = $state(true);

	let certLifetimeDays: number = $state(7);
	let certRenewalWindowHours: number = $state(6);

	let enrollmentConfigured: boolean = $state(false);
	let generatedToken: string | null = $state(null);

	let oidcProviders: OidcProviderResponse[] = $state([]);

	// --- MQTT Client ---
	let mqttConfigured: boolean = $state(false);
	let mqttEnabled: boolean = $state(true);
	let mqttUrl: string = $state('');
	let mqttClientId: string = $state('uptrakit-controller');
	let mqttUsername: string = $state('');
	let mqttPassword: string = $state('');
	let mqttHasPassword: boolean = $state(false);
	let mqttTopicPrefix: string = $state('uptrakit');
	let mqttDeleteConfirm: boolean = $state(false);

	// --- OIDC modal state ---
	let showOidcModal: boolean = $state(false);
	let editingProvider: OidcProviderResponse | null = $state(null);
	let oidcForm = $state({
		name: '',
		slug: '',
		logo_url: '',
		issuer_url: '',
		client_id: '',
		client_secret: '',
		scopes: 'openid email profile groups',
		auto_create_users: true,
		role_claim_path: '',
		role_mapping_json: '{}'
	});

	let slugTouched: boolean = $state(false);

	function slugify(text: string): string {
		return text
			.toLowerCase()
			.trim()
			.replace(/[^a-z0-9]+/g, '-')
			.replace(/^-+|-+$/g, '');
	}

	function onOidcNameInput() {
		if (!editingProvider && !slugTouched) {
			oidcForm.slug = slugify(oidcForm.name);
		}
	}

	// --- Delete confirmation ---
	let deleteConfirm: { id: string; name: string } | null = $state(null);

	// --- Loading ---
	let loading: boolean = $state(true);

	const canManageSettings = $derived($user?.permissions.includes(Permission.ManageSettings) ?? false);

	$effect(() => {
		if (!$user) {
			goto('/login');
		} else if (!canManageSettings) {
			goto('/');
		}
	});

	$effect(() => {
		if (canManageSettings) {
			loadAllSettings();
		}
	});

	function showSuccess(msg: string) {
		successMessage = msg;
		setTimeout(() => {
			successMessage = null;
		}, 3000);
	}

	function showError(msg: string) {
		errorMessage = msg;
	}

	function clearError() {
		errorMessage = null;
	}

	async function loadAllSettings() {
		loading = true;
		const results = await Promise.allSettled([
			getRegistrationSettings(),
			getAuthenticationSettings(),
			getAgentCertificateSettings(),
			getEnrollmentTokenStatus(),
			getOidcProviders(),
			getMqttClient().catch((e) => {
				// 404 means no MQTT client configured — not an error
				if (e instanceof Error && e.message === 'Not Found') return null;
				throw e;
			})
		]);

		if (results[0].status === 'fulfilled') {
			regMode = results[0].value.mode;
			regRequireTokenForOidc = results[0].value.require_token_for_oidc;
		}
		if (results[1].status === 'fulfilled') {
			passwordAuthEnabled = results[1].value.password_auth_enabled;
		}
		if (results[2].status === 'fulfilled') {
			certLifetimeDays = results[2].value.lifetime_days;
			certRenewalWindowHours = results[2].value.renewal_window_hours;
		}
		if (results[3].status === 'fulfilled') {
			enrollmentConfigured = results[3].value.configured;
		}
		if (results[4].status === 'fulfilled') {
			oidcProviders = results[4].value;
		}
		if (results[5].status === 'fulfilled') {
			const mqtt = results[5].value;
			if (mqtt) {
				mqttConfigured = true;
				mqttEnabled = mqtt.enabled;
				mqttUrl = mqtt.url;
				mqttClientId = mqtt.client_id;
				mqttUsername = mqtt.username ?? '';
				mqttHasPassword = mqtt.has_password;
				mqttTopicPrefix = mqtt.topic_prefix;
				mqttPassword = '';
			} else {
				mqttConfigured = false;
			}
		}

		loading = false;
	}

	// --- MQTT Client ---
	function applyMqttResponse(res: MqttClientResponse) {
		mqttConfigured = true;
		mqttEnabled = res.enabled;
		mqttUrl = res.url;
		mqttClientId = res.client_id;
		mqttUsername = res.username ?? '';
		mqttHasPassword = res.has_password;
		mqttTopicPrefix = res.topic_prefix;
		mqttPassword = '';
	}

	async function saveMqttClient() {
		clearError();
		try {
			if (mqttConfigured) {
				const data: Record<string, unknown> = {
					url: mqttUrl || undefined,
					enabled: mqttEnabled,
					client_id: mqttClientId,
					username: mqttUsername || null,
					topic_prefix: mqttTopicPrefix
				};
				if (mqttPassword) {
					data.password = mqttPassword;
				}
				const res = await updateMqttClient(data);
				applyMqttResponse(res);
			} else {
				if (!mqttUrl) {
					showError('URL is required to create an MQTT client');
					return;
				}
				const data: Record<string, unknown> = {
					url: mqttUrl,
					enabled: mqttEnabled,
					client_id: mqttClientId || undefined,
					username: mqttUsername || undefined,
					password: mqttPassword || undefined,
					topic_prefix: mqttTopicPrefix || undefined
				};
				const res = await createMqttClient(data);
				applyMqttResponse(res);
			}
			showSuccess('MQTT settings saved.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save MQTT settings');
		}
	}

	async function handleDeleteMqtt() {
		clearError();
		mqttDeleteConfirm = false;
		try {
			await deleteMqttClient();
			mqttConfigured = false;
			mqttEnabled = true;
			mqttUrl = '';
			mqttClientId = 'uptrakit-controller';
			mqttUsername = '';
			mqttPassword = '';
			mqttHasPassword = false;
			mqttTopicPrefix = 'uptrakit';
			showSuccess('MQTT client deleted.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete MQTT client');
		}
	}

	// --- Registration ---
	async function saveRegistration() {
		clearError();
		try {
			const data: { mode: 'open' | 'invite' | 'closed'; token?: string; require_token_for_oidc?: boolean } = { mode: regMode };
			if (regMode === 'invite' && regToken) {
				data.token = regToken;
			}
			if (regMode === 'invite') {
				data.require_token_for_oidc = regRequireTokenForOidc;
			}
			const res = await updateRegistrationSettings(data);
			regMode = res.mode;
			regRequireTokenForOidc = res.require_token_for_oidc;
			regToken = '';
			showSuccess('Registration settings saved.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save registration settings');
		}
	}

	// --- Authentication ---
	async function saveAuthentication() {
		clearError();
		try {
			const res = await updateAuthenticationSettings({
				password_auth_enabled: passwordAuthEnabled
			});
			passwordAuthEnabled = res.password_auth_enabled;
			showSuccess('Authentication settings saved.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save authentication settings');
		}
	}

	// --- Agent Certificates ---
	async function saveCertificates() {
		clearError();
		try {
			const res = await updateAgentCertificateSettings({
				lifetime_days: certLifetimeDays,
				renewal_window_hours: certRenewalWindowHours
			});
			certLifetimeDays = res.lifetime_days;
			certRenewalWindowHours = res.renewal_window_hours;
			showSuccess('Agent certificate settings saved.');
		} catch (e) {
			showError(
				e instanceof Error ? e.message : 'Failed to save agent certificate settings'
			);
		}
	}

	// --- Enrollment Token ---
	async function handleGenerateToken() {
		clearError();
		try {
			const res = await createEnrollmentToken();
			generatedToken = res.token;
			enrollmentConfigured = true;
			showSuccess('Enrollment token generated.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to generate enrollment token');
		}
	}

	async function handleRevokeToken() {
		clearError();
		try {
			await revokeEnrollmentToken();
			enrollmentConfigured = false;
			generatedToken = null;
			showSuccess('Enrollment token revoked.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to revoke enrollment token');
		}
	}

	// --- OIDC Providers ---
	function openCreateOidc() {
		editingProvider = null;
		slugTouched = false;
		oidcForm = {
			name: '',
			slug: '',
			logo_url: '',
			issuer_url: '',
			client_id: '',
			client_secret: '',
			scopes: 'openid email profile groups',
			auto_create_users: true,
			role_claim_path: '',
			role_mapping_json: '{}'
		};
		showOidcModal = true;
	}

	function openEditOidc(provider: OidcProviderResponse) {
		editingProvider = provider;
		oidcForm = {
			name: provider.name,
			slug: provider.slug,
			logo_url: provider.logo_url ?? '',
			issuer_url: provider.issuer_url,
			client_id: provider.client_id,
			client_secret: '',
			scopes: provider.scopes,
			auto_create_users: provider.auto_create_users,
			role_claim_path: provider.role_claim_path ?? '',
			role_mapping_json: JSON.stringify(provider.role_mapping, null, 2)
		};
		showOidcModal = true;
	}

	function closeOidcModal() {
		showOidcModal = false;
		editingProvider = null;
	}

	async function saveOidcProvider() {
		clearError();

		let roleMapping: Record<string, string>;
		try {
			roleMapping = JSON.parse(oidcForm.role_mapping_json);
		} catch {
			showError('Role mapping must be valid JSON (e.g. {"oidc_value": "local_role"})');
			return;
		}

		try {
			if (editingProvider) {
				const data: UpdateOidcProviderRequest = {
					name: oidcForm.name,
					slug: oidcForm.slug,
					issuer_url: oidcForm.issuer_url,
					client_id: oidcForm.client_id,
					scopes: oidcForm.scopes,
					auto_create_users: oidcForm.auto_create_users,
					role_mapping: roleMapping
				};
				if (oidcForm.logo_url) data.logo_url = oidcForm.logo_url;
				if (oidcForm.client_secret) data.client_secret = oidcForm.client_secret;
				if (oidcForm.role_claim_path) data.role_claim_path = oidcForm.role_claim_path;

				const updated = await updateOidcProvider(editingProvider.id, data);
				oidcProviders = oidcProviders.map((p) =>
					p.id === updated.id ? updated : p
				);
				showSuccess('OIDC provider updated.');
			} else {
				const data: CreateOidcProviderRequest = {
					name: oidcForm.name,
					slug: oidcForm.slug,
					issuer_url: oidcForm.issuer_url,
					client_id: oidcForm.client_id,
					client_secret: oidcForm.client_secret,
					scopes: oidcForm.scopes,
					auto_create_users: oidcForm.auto_create_users,
					role_mapping: roleMapping
				};
				if (oidcForm.logo_url) data.logo_url = oidcForm.logo_url;
				if (oidcForm.role_claim_path) data.role_claim_path = oidcForm.role_claim_path;

				const created = await createOidcProvider(data);
				oidcProviders = [...oidcProviders, created];
				showSuccess('OIDC provider created.');
			}
			closeOidcModal();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save OIDC provider');
		}
	}

	function requestDeleteOidc(provider: OidcProviderResponse) {
		deleteConfirm = { id: provider.id, name: provider.name };
	}

	async function executeDeleteOidc() {
		if (!deleteConfirm) return;
		clearError();
		const { id } = deleteConfirm;
		deleteConfirm = null;
		try {
			await deleteOidcProvider(id);
			oidcProviders = oidcProviders.filter((p) => p.id !== id);
			showSuccess('OIDC provider deleted.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete OIDC provider');
		}
	}

	async function toggleOidcActive(provider: OidcProviderResponse) {
		clearError();
		try {
			let updated: OidcProviderResponse;
			if (provider.is_active) {
				updated = await deactivateOidcProvider(provider.id);
			} else {
				updated = await activateOidcProvider(provider.id);
			}
			// Activation may deactivate others, so reload all
			oidcProviders = await getOidcProviders();
			showSuccess(
				updated.is_active
					? `${updated.name} activated.`
					: `${updated.name} deactivated.`
			);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update provider status');
		}
	}
</script>

{#if $user && canManageSettings}
	<h1 class="h1 mb-6">Settings</h1>

	{#if successMessage}
		<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
			<p>{successMessage}</p>
		</aside>
	{/if}

	{#if errorMessage}
		<aside class="mb-4 flex items-center justify-between rounded-lg p-4 preset-filled-error-500">
			<p>{errorMessage}</p>
			<button class="btn btn-sm preset-filled" onclick={clearError}>Dismiss</button>
		</aside>
	{/if}

	{#if loading}
		<div class="card p-8 text-center">
			<p>Loading settings...</p>
		</div>
	{:else}
		<!-- Section 1: Registration -->
		<div class="card mb-6 p-6">
			<h2 class="h3 mb-4">Registration</h2>
			<label class="label mb-4">
				<span>Registration Mode</span>
				<select class="select" bind:value={regMode}>
					<option value="open">Open</option>
					<option value="invite">Invite Only</option>
					<option value="closed">Closed</option>
				</select>
			</label>

			{#if regMode === 'invite'}
				<label class="label mb-4">
					<span>Registration Token</span>
					<input
						class="input"
						type="text"
						placeholder="Enter a new registration token"
						bind:value={regToken}
					/>
					<small class="text-surface-600 dark:text-surface-400">Set a new token for invite-only registration. Leave blank to keep the current token.</small>
				</label>

				<label class="mb-4 flex items-center gap-3">
					<input class="checkbox" type="checkbox" bind:checked={regRequireTokenForOidc} />
					<span>Require registration token for OIDC users</span>
				</label>
				<small class="mb-4 block text-surface-600 dark:text-surface-400">When enabled, users signing in via OIDC for the first time must also provide the registration token.</small>
			{/if}

			<button class="btn preset-filled-primary-500" onclick={saveRegistration}>
				Save
			</button>
		</div>

		<!-- Section 2: Authentication -->
		<div class="card mb-6 p-6">
			<h2 class="h3 mb-4">Authentication</h2>
			<label class="mb-4 flex items-center gap-3">
				<input class="checkbox" type="checkbox" bind:checked={passwordAuthEnabled} />
				<span>Enable password authentication</span>
			</label>
			<button class="btn preset-filled-primary-500" onclick={saveAuthentication}>
				Save
			</button>
		</div>

		<!-- Section 3: MQTT Client -->
		<div class="card mb-6 p-6">
			<div class="mb-4 flex items-center justify-between">
				<h2 class="h3">MQTT Client</h2>
				{#if mqttConfigured}
					<span class="badge preset-filled-success-500">Configured</span>
				{:else}
					<span class="badge preset-tonal">Not configured</span>
				{/if}
			</div>
			<p class="mb-4 text-surface-600 dark:text-surface-400">
				Configure MQTT broker connection for Home Assistant integration.
				Use a URL like <code>mqtt://broker:1883</code> or <code>mqtts://broker:8883</code>.
				All MQTT changes require a restart to take effect.
			</p>

			<label class="mb-4 flex items-center gap-3">
				<input class="checkbox" type="checkbox" bind:checked={mqttEnabled} />
				<span>Enabled</span>
			</label>

			<label class="label mb-4">
				<span>Broker URL <span class="badge preset-tonal-warning ml-2 text-xs">Requires restart</span></span>
				<input class="input" type="text" placeholder="e.g. mqtt://broker:1883" bind:value={mqttUrl} />
			</label>

			<div class="mb-4 grid grid-cols-1 gap-4 sm:grid-cols-2">
				<label class="label">
					<span>Client ID <span class="badge preset-tonal-warning ml-2 text-xs">Requires restart</span></span>
					<input class="input" type="text" bind:value={mqttClientId} />
				</label>
				<label class="label">
					<span>Topic Prefix <span class="badge preset-tonal-warning ml-2 text-xs">Requires restart</span></span>
					<input class="input" type="text" bind:value={mqttTopicPrefix} />
				</label>
			</div>

			<div class="mb-4 grid grid-cols-1 gap-4 sm:grid-cols-2">
				<label class="label">
					<span>Username</span>
					<input class="input" type="text" placeholder="(optional)" bind:value={mqttUsername} />
				</label>
				<label class="label">
					<span>
						Password
						{#if mqttHasPassword}
							<span class="badge preset-filled-success-500 ml-2 text-xs">Password set</span>
						{/if}
					</span>
					<input
						class="input"
						type="password"
						placeholder="Leave blank to keep current"
						bind:value={mqttPassword}
					/>
				</label>
			</div>

			<div class="flex gap-2">
				<button class="btn preset-filled-primary-500" onclick={saveMqttClient}>
					Save
				</button>
				{#if mqttConfigured}
					<button class="btn preset-filled-error-500" onclick={() => { mqttDeleteConfirm = true; }}>
						Delete
					</button>
				{/if}
			</div>
		</div>

		<!-- MQTT Delete confirmation -->
		{#if mqttDeleteConfirm}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
				onkeydown={(e) => { if (e.key === 'Escape') { mqttDeleteConfirm = false; } }}
			>
				<div class="card w-full max-w-md space-y-4 p-6 shadow-xl">
					<h3 class="h3">Delete MQTT Client</h3>
					<p>Are you sure you want to remove the MQTT client configuration?</p>
					<div class="flex justify-end gap-2">
						<button class="btn preset-tonal-surface" onclick={() => { mqttDeleteConfirm = false; }}>
							Cancel
						</button>
						<button class="btn preset-filled-error-500" onclick={handleDeleteMqtt}>
							Delete
						</button>
					</div>
				</div>
			</div>
		{/if}

		<!-- Section 5: OIDC Providers -->
		<div class="card mb-6 p-6">
			<div class="mb-4 flex items-center justify-between">
				<h2 class="h3">OIDC Providers</h2>
				<button class="btn preset-filled-primary-500" onclick={openCreateOidc}>
					Add Provider
				</button>
			</div>

			{#if oidcProviders.length === 0}
				<p class="py-4 text-center text-surface-600 dark:text-surface-400">No OIDC providers configured.</p>
			{:else}
				<div class="table-wrap">
					<table class="table">
						<thead>
							<tr>
								<th>Name</th>
								<th>Slug</th>
								<th>Status</th>
								<th class="w-48">Actions</th>
							</tr>
						</thead>
						<tbody>
							{#each oidcProviders as provider (provider.id)}
								<tr>
									<td>{provider.name}</td>
									<td>{provider.slug}</td>
									<td>
										{#if provider.is_active}
											<span class="badge preset-filled-success-500">Active</span>
										{:else}
											<span class="badge preset-tonal">Inactive</span>
										{/if}
									</td>
									<td>
										<div class="flex gap-1">
											<button
												class="btn btn-sm preset-tonal"
												onclick={() => openEditOidc(provider)}
											>
												Edit
											</button>
											{#if provider.is_active}
												<button
													class="btn btn-sm preset-tonal-warning"
													onclick={() => toggleOidcActive(provider)}
												>
													Deactivate
												</button>
											{:else}
												<button
													class="btn btn-sm preset-tonal-success"
													onclick={() => toggleOidcActive(provider)}
												>
													Activate
												</button>
											{/if}
											<button
												class="btn btn-sm preset-tonal-error"
												onclick={() => requestDeleteOidc(provider)}
											>
												Delete
											</button>
										</div>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</div>

		<!-- Section 4: Agent Certificates -->
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

		<!-- Section 5: Enrollment Token -->
		<div class="card mb-6 p-6">
			<h2 class="h3 mb-4">Enrollment Token</h2>
			<div class="mb-4 flex items-center gap-3">
				<span>Status:</span>
				{#if enrollmentConfigured}
					<span class="badge preset-filled-success-500">Configured</span>
				{:else}
					<span class="badge preset-tonal">Not configured</span>
				{/if}
			</div>

			{#if generatedToken}
				<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
					<p class="font-bold">Copy it now — it will not be shown again</p>
					<code class="break-all">{generatedToken}</code>
				</aside>
			{/if}

			<div class="flex gap-2">
				<button class="btn preset-filled-primary-500" onclick={handleGenerateToken}>
					{enrollmentConfigured ? 'Regenerate' : 'Generate'}
				</button>
				{#if enrollmentConfigured}
					<button class="btn preset-filled-error-500" onclick={handleRevokeToken}>
						Revoke
					</button>
				{/if}
			</div>
		</div>
	{/if}

	<!-- OIDC Provider Modal -->
	{#if showOidcModal}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
			onkeydown={(e) => { if (e.key === 'Escape') closeOidcModal(); }}
		>
			<div class="card w-full max-w-2xl max-h-[90vh] space-y-4 overflow-y-auto p-6 shadow-xl">
				<h3 class="h3">{editingProvider ? 'Edit OIDC Provider' : 'Add OIDC Provider'}</h3>

				<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
					<label class="label">
						<span>Name</span>
						<input class="input" type="text" bind:value={oidcForm.name} oninput={onOidcNameInput} />
					</label>
					<label class="label">
						<span>Slug</span>
						<input class="input" type="text" bind:value={oidcForm.slug} oninput={() => { slugTouched = true; }} />
					</label>
				</div>

				<label class="label">
					<span>Logo URL</span>
					<input class="input" type="text" placeholder="https://..." bind:value={oidcForm.logo_url} />
				</label>

				<label class="label">
					<span>Issuer URL</span>
					<input class="input" type="text" bind:value={oidcForm.issuer_url} />
				</label>

				<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
					<label class="label">
						<span>Client ID</span>
						<input class="input" type="text" bind:value={oidcForm.client_id} />
					</label>
					<label class="label">
						<span>Client Secret</span>
						<input
							class="input"
							type="password"
							placeholder={editingProvider ? 'Leave blank to keep current' : ''}
							bind:value={oidcForm.client_secret}
						/>
					</label>
				</div>

				<label class="label">
					<span>Scopes</span>
					<input class="input" type="text" bind:value={oidcForm.scopes} />
				</label>

				<label class="flex items-center gap-3">
					<input class="checkbox" type="checkbox" bind:checked={oidcForm.auto_create_users} />
					<span>Auto-create users on first login</span>
				</label>

				<label class="label">
					<span>Role Claim Path</span>
					<input class="input" type="text" placeholder="e.g. groups" bind:value={oidcForm.role_claim_path} />
				</label>

				<label class="label">
					<span>Role Mapping (JSON)</span>
					<textarea
						class="textarea"
						rows="3"
						placeholder={'{"oidc_value": "local_role"}'}
						bind:value={oidcForm.role_mapping_json}
					></textarea>
				</label>

				<div class="flex justify-end gap-2">
					<button class="btn preset-tonal-surface" onclick={closeOidcModal}>Cancel</button>
					<button class="btn preset-filled-primary-500" onclick={saveOidcProvider}>
						{editingProvider ? 'Update' : 'Create'}
					</button>
				</div>
			</div>
		</div>
	{/if}

	<!-- Delete confirmation modal -->
	{#if deleteConfirm}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
			onkeydown={(e) => { if (e.key === 'Escape') { deleteConfirm = null; } }}
		>
			<div class="card w-full max-w-md space-y-4 p-6 shadow-xl">
				<h3 class="h3">Delete OIDC Provider</h3>
				<p>
					Are you sure you want to delete
					<strong>{deleteConfirm.name}</strong>?
				</p>
				<div class="flex justify-end gap-2">
					<button class="btn preset-tonal-surface" onclick={() => { deleteConfirm = null; }}>
						Cancel
					</button>
					<button class="btn preset-filled-error-500" onclick={executeDeleteOidc}>
						Delete
					</button>
				</div>
			</div>
		</div>
	{/if}
{/if}
