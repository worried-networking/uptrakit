<script lang="ts">
	import { getUser } from '$lib/auth.svelte';
	import {
		getGitHubProviderSettings,
		getSystemAlerts,
		renewServerCertificate,
		getNetworkSettings,
		updateNetworkSettings,
		getNatsSettings,
		updateNatsSettings,
		updateGitHubProviderSettings,
		getZeroconfSettings,
		updateZeroconfSettings,
		rotateCA
	} from '$lib/api';
	import { Permission, hasAnyPermission, hasPermissionValue, type SystemAlert } from '$lib/types';
	import { showSuccess, showError, clearError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import SystemServicesSettings from './SystemServicesSettings.svelte';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import { Callout, FormFieldRow, SectionCard } from '$lib/components/ui';
	import {
		getSurfaceReadModel,
		getSurfaceRuntimeStatus,
		getSurfacesBySlot,
		loadSurfaceReadModels
	} from '$lib/surfaces/registry.svelte';
	import { filterSurfacesByPermission, shouldUseSurfaceRoute } from '$lib/surfaces/read-model';

	// --- Network Settings ---
	let trustedProxiesText: string = $state('');
	let realIpHeader: string = $state('X-Forwarded-For');
	let sansText: string = $state('');
	let regenerateCert: boolean = $state(false);
	let httpsAddr: string = $state('[::]:8443');

	// --- TLS Certificate ---
	let tlsAlerts: SystemAlert[] = $state([]);
	let renewingCert: boolean = $state(false);

	// --- CA Certificate ---
	let showRotateCaConfirm: boolean = $state(false);
	let rotatingCa: boolean = $state(false);

	// --- NATS Settings ---
	let natsAvailable: boolean = $state(false);
	let natsCurrentUrl: string | null = $state(null);
	let natsUrlInput: string = $state('');
	let natsSaving: boolean = $state(false);
	let natsClearing: boolean = $state(false);

	// --- Global GitHub Provider Settings ---
	let githubProviderAvailable: boolean = $state(false);
	let githubProviderApiBaseUrl: string = $state('');
	let githubProviderAuthToken: string = $state('');
	let githubProviderHasAuthToken: boolean = $state(false);
	let githubProviderSaving: boolean = $state(false);

	// --- Zeroconf Settings ---
	let zeroconfAvailable: boolean = $state(false);
	let zeroconfEnabled: boolean = $state(false);
	let zeroconfCaFingerprint: string | null = $state(null);
	let zeroconfUrlOverride: string = $state('');
	let zeroconfPkiAddrOverride: string = $state('');
	let zeroconfSaving: boolean = $state(false);

	// --- Loading ---
	let loading: boolean = $state(true);

	const belowSurfaces = $derived(
		filterSurfacesByPermission(getSurfacesBySlot('settings.below.global'), (requiredPermission) =>
			hasPermissionValue(getUser(), requiredPermission)
		)
	);
	const belowSurfaceReads = $derived.by(() => {
		const result: Record<string, NonNullable<ReturnType<typeof getSurfaceReadModel>>> = {};
		for (const surface of belowSurfaces) {
			const read = getSurfaceReadModel(surface.surface_id);
			if (read) {
				result[surface.surface_id] = read;
			}
		}
		return result;
	});
	const useSurfaceBelowPanels = $derived(
		shouldUseSurfaceRoute(getSurfaceRuntimeStatus().active, belowSurfaces, belowSurfaceReads)
	);

	const canManageSystemServices = $derived(
		hasAnyPermission(
			getUser(),
			Permission.ApproveSystemServices,
			Permission.RejectSystemServices,
			Permission.RemoveSystemServices,
			Permission.UpdateSystemServices
		)
	);

	$effect(() => {
		loadGlobalSettings();
	});

	$effect(() => {
		if (!getSurfaceRuntimeStatus().active || belowSurfaces.length === 0) {
			return;
		}
		void loadSurfaceReadModels(belowSurfaces.map((surface) => surface.surface_id));
	});

	async function loadGlobalSettings() {
		loading = true;
		const results = await Promise.allSettled([
			getNetworkSettings(),
			getSystemAlerts(),
			getNatsSettings(),
			getZeroconfSettings(),
			getGitHubProviderSettings()
		]);

		if (results[0].status === 'fulfilled') {
			const net = results[0].value;
			trustedProxiesText = net.trusted_proxies.join('\n');
			realIpHeader = net.real_ip_header;
			sansText = net.sans.join('\n');
			httpsAddr = net.https_addr;
		}
		if (results[1].status === 'fulfilled') {
			tlsAlerts = results[1].value.alerts.filter(
				(a) => a.id === 'server_cert_old_ca' || a.id === 'server_cert_expiring'
			);
		}
		if (results[2].status === 'fulfilled') {
			natsAvailable = true;
			natsCurrentUrl = results[2].value.url ?? null;
		} else {
			// 404 means the NATS feature is not compiled in — hide the section gracefully
			natsAvailable = false;
		}
		if (results[3].status === 'fulfilled') {
			zeroconfAvailable = true;
			const zc = results[3].value;
			zeroconfEnabled = zc.enabled;
			zeroconfCaFingerprint = zc.ca_fingerprint ?? null;
			zeroconfUrlOverride = zc.url ?? '';
			zeroconfPkiAddrOverride = zc.pki_addr ?? '';
		} else {
			// 404 means the zeroconf feature is not compiled in — hide the section gracefully
			zeroconfAvailable = false;
		}
		if (results[4].status === 'fulfilled') {
			githubProviderAvailable = true;
			const github = results[4].value;
			githubProviderApiBaseUrl = github.api_base_url ?? '';
			githubProviderAuthToken = github.auth_token ?? '';
			githubProviderHasAuthToken = github.has_auth_token;
		} else {
			githubProviderAvailable = false;
		}

		loading = false;
	}

	// --- NATS Settings ---
	async function saveNatsUrl() {
		clearError();
		natsSaving = true;
		try {
			const res = await updateNatsSettings({ url: natsUrlInput.trim() || null });
			natsCurrentUrl = res.url ?? null;
			natsUrlInput = '';
			showSuccess('NATS URL updated. Changes take effect after the controller is restarted.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save NATS URL');
		} finally {
			natsSaving = false;
		}
	}

	async function clearNatsUrl() {
		clearError();
		natsClearing = true;
		try {
			const res = await updateNatsSettings({ url: null });
			natsCurrentUrl = res.url ?? null;
			natsUrlInput = '';
			showSuccess('NATS URL cleared. Changes take effect after the controller is restarted.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to clear NATS URL');
		} finally {
			natsClearing = false;
		}
	}

	// --- Global GitHub Provider Settings ---
	async function saveGitHubProviderSettings() {
		clearError();
		githubProviderSaving = true;
		try {
			const res = await updateGitHubProviderSettings({
				auth_token: githubProviderAuthToken.trim(),
				api_base_url: githubProviderApiBaseUrl.trim()
			});
			githubProviderApiBaseUrl = res.api_base_url ?? '';
			githubProviderAuthToken = res.auth_token ?? '';
			githubProviderHasAuthToken = res.has_auth_token;
			showSuccess('Global GitHub provider settings saved.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save GitHub provider settings');
		} finally {
			githubProviderSaving = false;
		}
	}

	// --- Zeroconf Settings ---
	async function saveZeroconfSettings() {
		clearError();
		zeroconfSaving = true;
		try {
			const res = await updateZeroconfSettings({
				enabled: zeroconfEnabled,
				url: zeroconfUrlOverride.trim() || undefined,
				pki_addr: zeroconfPkiAddrOverride.trim() || undefined
			});
			zeroconfEnabled = res.enabled;
			zeroconfCaFingerprint = res.ca_fingerprint ?? null;
			zeroconfUrlOverride = res.url ?? '';
			zeroconfPkiAddrOverride = res.pki_addr ?? '';
			showSuccess(
				'Zero-configuration discovery settings saved. Changes take effect after the controller is restarted.'
			);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save zeroconf settings');
		} finally {
			zeroconfSaving = false;
		}
	}

	// --- Network Settings ---
	async function saveNetworkSettings() {
		clearError();
		try {
			const proxies = trustedProxiesText
				.split('\n')
				.map((s) => s.trim())
				.filter((s) => s.length > 0);
			const sans = sansText
				.split('\n')
				.map((s) => s.trim())
				.filter((s) => s.length > 0);
			const res = await updateNetworkSettings({
				trusted_proxies: proxies,
				real_ip_header: realIpHeader,
				sans: sans,
				https_addr: httpsAddr,
				regenerate_cert: regenerateCert || undefined
			});
			trustedProxiesText = res.trusted_proxies.join('\n');
			realIpHeader = res.real_ip_header;
			sansText = res.sans.join('\n');
			httpsAddr = res.https_addr;
			regenerateCert = false;
			if (res.cert_regenerated) {
				showSuccess('Network settings saved. Server certificate regenerated.');
			} else {
				showSuccess('Network settings saved.');
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save network settings');
		}
	}

	// --- CA Certificate ---
	async function handleRotateCa() {
		showRotateCaConfirm = false;
		rotatingCa = true;
		try {
			await rotateCA();
			showSuccess('CA certificate rotated. All agents must re-enroll.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to rotate CA certificate');
		} finally {
			rotatingCa = false;
		}
	}

	// --- Server Certificate ---
	async function handleRenewServerCert() {
		clearError();
		renewingCert = true;
		try {
			await renewServerCertificate();
			tlsAlerts = tlsAlerts.filter((a) => a.id !== 'server_cert_old_ca' && a.id !== 'server_cert_expiring');
			showSuccess('Server certificate renewed successfully.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to renew server certificate');
		} finally {
			renewingCert = false;
		}
	}
</script>

{#if loading}
	<SectionCard title="Global Settings" description="Loading controller-wide configuration panels.">
		<p class="text-sm text-[var(--text-secondary)]">Loading global settings...</p>
	</SectionCard>
{:else}
	<!-- Section 1: Global GitHub Provider -->
	{#if githubProviderAvailable}
		<SectionCard
			title="GitHub Provider"
			description="Shared GitHub settings for controller-managed global plugins such as Dashboard Icons."
		>
			<div class="mb-4 grid gap-3 md:grid-cols-2" data-ui="github-provider-summary">
				<article class="rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2">
					<p class="text-[7.5px] uppercase tracking-[0.12em] text-[var(--text-secondary)]">Request mode</p>
					<p class="mt-1 text-[14px] font-semibold text-[var(--text-primary)]">
						{githubProviderHasAuthToken ? 'Authenticated' : 'Anonymous fallback'}
					</p>
				</article>
				<article class="rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2">
					<p class="text-[7.5px] uppercase tracking-[0.12em] text-[var(--text-secondary)]">API endpoint</p>
					<p class="mt-1 break-all font-mono text-[13px] text-[var(--text-primary)]">
						{githubProviderApiBaseUrl || 'https://api.github.com'}
					</p>
				</article>
			</div>

			<div class="mb-4 space-y-3">
				<Callout
					tone="info"
					title="Global plugins only"
					message="These credentials are used only by controller-managed global plugins. Tenant-scoped release plugins keep using their own plugin configuration."
				/>
				<Callout
					tone="warning"
					title="Anonymous fallback stays available"
					message="Leaving the token blank keeps GitHub access unauthenticated. Setting a token improves the shared rate-limit budget for global plugin traffic."
				/>
			</div>

			<div class="space-y-4">
				<FormFieldRow
					label="API Base URL"
					hint="Optional. Leave blank to use the public GitHub API. Use a full HTTPS API base URL for GitHub Enterprise."
					inputId="global-github-provider-api-base-url"
				>
					<input
						id="global-github-provider-api-base-url"
						class="input font-mono"
						type="text"
						placeholder="https://ghe.example.com/api/v3"
						bind:value={githubProviderApiBaseUrl}
					/>
				</FormFieldRow>

				<FormFieldRow
					label="Auth Token"
					hint="Optional. Keep the masked value to preserve the current token, replace it to rotate, or clear the field to remove it."
					inputId="global-github-provider-auth-token"
				>
					<input
						id="global-github-provider-auth-token"
						class="input font-mono"
						type="password"
						placeholder="Leave blank for anonymous requests"
						bind:value={githubProviderAuthToken}
					/>
				</FormFieldRow>

				<div class="flex flex-wrap gap-2">
					<button
						class="btn preset-filled-primary-500"
						onclick={saveGitHubProviderSettings}
						disabled={githubProviderSaving}
					>
						{githubProviderSaving ? 'Saving…' : 'Save GitHub Provider'}
					</button>
				</div>
			</div>
		</SectionCard>
	{/if}

	<!-- Section 2: NATS Configuration -->
	{#if natsAvailable}
		<SectionCard title="NATS Configuration">
			<p class="mb-4 text-surface-600 dark:text-surface-400">
				Configure the NATS server URL used for inter-service messaging. The URL may include embedded credentials (e.g. <code
					>nats://user:password@host:4222</code
				>).
			</p>

			<aside class="mb-4 rounded-lg bg-surface-100-900 p-3 text-sm">
				<strong>Requires restart:</strong> Changes to the NATS URL take effect after the controller is restarted.
			</aside>

			<FormFieldRow label="Current URL">
				<p class="font-mono text-sm text-surface-700 dark:text-surface-300">{natsCurrentUrl ?? '— not configured —'}</p>
			</FormFieldRow>

			<FormFieldRow label="New NATS URL" inputId="global-nats-url">
				<input
					id="global-nats-url"
					class="input font-mono"
					type="text"
					placeholder="nats://host:4222"
					bind:value={natsUrlInput}
				/>
			</FormFieldRow>

			<div class="flex gap-2">
				<button
					class="btn preset-filled-primary-500"
					onclick={saveNatsUrl}
					disabled={natsSaving || !natsUrlInput.trim()}
				>
					{natsSaving ? 'Saving…' : 'Save'}
				</button>
				{#if natsCurrentUrl}
					<button class="btn preset-tonal-error" onclick={clearNatsUrl} disabled={natsClearing}>
						{natsClearing ? 'Clearing…' : 'Clear'}
					</button>
				{/if}
			</div>
		</SectionCard>
	{/if}

	<!-- Section 3: Zero-Configuration Discovery -->
	{#if zeroconfAvailable}
		<SectionCard title="Zero-Configuration Discovery">
			<p class="mb-4 text-surface-600 dark:text-surface-400">
				When enabled, the controller advertises itself on the local network via mDNS (Bonjour/Avahi), allowing agents to
				discover and enroll without manual URL configuration. Use the override fields below for reverse proxy or
				split-network deployments where the advertised addresses differ from the controller's local addresses.
			</p>

			<aside class="mb-4 rounded-lg bg-surface-100-900 p-3 text-sm">
				<strong>Requires restart:</strong> Changes to these settings take effect after the controller is restarted.
			</aside>

			<FormFieldRow label="mDNS Advertising" inputId="global-zeroconf-enabled">
				<label class="flex items-center gap-2">
					<input id="global-zeroconf-enabled" class="checkbox" type="checkbox" bind:checked={zeroconfEnabled} />
					<span>Enable mDNS advertising</span>
					<span class="badge preset-tonal-warning ml-2 text-xs">Requires restart</span>
				</label>
			</FormFieldRow>

			{#if zeroconfCaFingerprint}
				<FormFieldRow label="CA Fingerprint">
					<p class="font-mono text-sm text-surface-700 dark:text-surface-300">{zeroconfCaFingerprint}</p>
				</FormFieldRow>
			{/if}

			<FormFieldRow label="URL Override" inputId="global-zeroconf-url-override">
				<input
					id="global-zeroconf-url-override"
					class="input font-mono"
					type="text"
					placeholder="https://proxy.example.com:443"
					bind:value={zeroconfUrlOverride}
				/>
			</FormFieldRow>

			<FormFieldRow label="PKI Address Override" inputId="global-zeroconf-pki-addr-override">
				<input
					id="global-zeroconf-pki-addr-override"
					class="input font-mono"
					type="text"
					placeholder="http://pki.local:8080"
					bind:value={zeroconfPkiAddrOverride}
				/>
			</FormFieldRow>

			<button class="btn preset-filled-primary-500" onclick={saveZeroconfSettings} disabled={zeroconfSaving}>
				{zeroconfSaving ? 'Saving...' : 'Save'}
			</button>
		</SectionCard>
	{/if}

	<!-- Section 4: Network Settings -->
	<SectionCard title="Network Settings">
		<p class="mb-4 text-surface-600 dark:text-surface-400">
			Configure reverse proxy trust, client IP detection, and listen addresses. Changes to listen addresses require a
			restart to take effect.
		</p>

		<FormFieldRow label="Trusted Proxies" hint="One IP/CIDR per line." inputId="global-trusted-proxies">
			<textarea class="textarea" rows="3" placeholder="e.g. 10.0.0.0/8&#10;192.168.1.1" bind:value={trustedProxiesText}
			></textarea>
		</FormFieldRow>

		<FormFieldRow label="Real IP Header" inputId="global-real-ip-header">
			<select id="global-real-ip-header" class="select" bind:value={realIpHeader}>
				<option value="X-Forwarded-For">X-Forwarded-For</option>
				<option value="Forwarded">Forwarded (RFC 7239)</option>
				<option value="X-Real-Ip">X-Real-Ip</option>
				<option value="CF-Connecting-IP">CF-Connecting-IP</option>
				<option value="True-Client-IP">True-Client-IP</option>
			</select>
		</FormFieldRow>

		<FormFieldRow
			label="Certificate SANs"
			hint="One IP or DNS name per line. Auto-detected on first startup; changes replace the full list."
			inputId="global-certificate-sans"
		>
			<textarea class="textarea" rows="3" placeholder="e.g. controller.local&#10;192.168.1.100" bind:value={sansText}
			></textarea>
		</FormFieldRow>

		<FormFieldRow label="Regenerate Certificate" inputId="global-regenerate-cert">
			<label class="flex items-center gap-2">
				<input id="global-regenerate-cert" type="checkbox" class="checkbox" bind:checked={regenerateCert} />
				<span>Regenerate server certificate after update</span>
			</label>
		</FormFieldRow>

		<FormFieldRow label="HTTPS Listen Address" inputId="global-https-addr">
			<div class="space-y-2">
				<div><span class="badge preset-tonal-warning text-xs">Requires restart</span></div>
				<input id="global-https-addr" class="input" type="text" bind:value={httpsAddr} />
			</div>
		</FormFieldRow>

		<button class="btn preset-filled-primary-500" onclick={saveNetworkSettings}> Save </button>
	</SectionCard>

	<!-- Section 5: Controller TLS Certificate -->
	<SectionCard title="Controller TLS Certificate">
		<p class="mb-4 text-surface-600 dark:text-surface-400">
			The controller's HTTPS certificate is automatically renewed before expiration. You can manually renew it here to
			re-issue under the current active CA.
		</p>

		{#if tlsAlerts.length > 0}
			{#each tlsAlerts as alert (alert.id)}
				<aside
					class="mb-4 rounded-lg p-4 {alert.severity === 'warning'
						? 'preset-filled-warning-500'
						: 'preset-filled-surface-400-600'}"
				>
					<p>{alert.message}</p>
				</aside>
			{/each}
		{/if}

		<button class="btn preset-filled-primary-500" onclick={handleRenewServerCert} disabled={renewingCert}>
			{renewingCert ? 'Renewing...' : 'Renew Server Certificate'}
		</button>
	</SectionCard>

	<!-- Section 6: System Services -->
	{#if canManageSystemServices}
		<SystemServicesSettings onSuccess={showSuccess} onError={showError} />
	{/if}

	<!-- Section 7: CA Certificate -->
	<SectionCard title="CA Certificate">
		<p class="mb-4 text-surface-600 dark:text-surface-400">
			Rotate the root CA certificate used to sign all agent and server certificates. This will invalidate all currently
			issued certificates and require all agents to re-enroll.
		</p>
		<button class="btn preset-filled-error-500" onclick={() => (showRotateCaConfirm = true)} disabled={rotatingCa}>
			{rotatingCa ? 'Rotating...' : 'Rotate CA'}
		</button>
	</SectionCard>

	{#if showRotateCaConfirm}
		<ConfirmDialog
			title="Rotate CA Certificate"
			messagePrefix="This will invalidate all existing agent certificates and require re-enrollment of"
			entityName="all agents. Are you sure?"
			confirmLabel={rotatingCa ? 'Rotating...' : 'Rotate CA'}
			confirmClass="preset-filled-error-500"
			confirmDisabled={rotatingCa}
			onconfirm={handleRotateCa}
			oncancel={() => (showRotateCaConfirm = false)}
		/>
	{/if}

	<!-- Extension panels positioned below global settings -->
	{#if useSurfaceBelowPanels}
		{#each belowSurfaces as surface (surface.surface_id)}
			<SectionCard title={surface.label}>
				<SurfaceReadPanel {surface} read={belowSurfaceReads[surface.surface_id]} />
			</SectionCard>
		{/each}
	{/if}
{/if}
