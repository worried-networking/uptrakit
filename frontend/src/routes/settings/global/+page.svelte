<script lang="ts">
	import { user } from '$lib/auth';
	import { goto } from '$app/navigation';
	import {
		getSystemAlerts,
		renewServerCertificate,
		getNetworkSettings,
		updateNetworkSettings
	} from '$lib/api';
	import {
		Permission,
		type SystemAlert
	} from '$lib/types';
	import { showSuccess, showError, clearError, getSuccessMessage, getErrorMessage } from '$lib/notifications.svelte';

	// --- Network Settings ---
	let trustedProxiesText: string = $state('');
	let realIpHeader: string = $state('X-Forwarded-For');
	let extraSansText: string = $state('');
	let httpsAddr: string = $state('[::]:8443');

	// --- TLS Certificate ---
	let tlsAlerts: SystemAlert[] = $state([]);
	let renewingCert: boolean = $state(false);

	// --- Loading ---
	let loading: boolean = $state(true);

	const canManageGlobalSettings = $derived($user?.permissions.includes(Permission.ManageGlobalSettings) ?? false);
	const successMessage = $derived(getSuccessMessage());
	const errorMessage = $derived(getErrorMessage());

	$effect(() => {
		if ($user && !canManageGlobalSettings) {
			goto('/');
		}
	});

	$effect(() => {
		if (canManageGlobalSettings) {
			loadGlobalSettings();
		}
	});

	async function loadGlobalSettings() {
		loading = true;
		const results = await Promise.allSettled([
			getNetworkSettings(),
			getSystemAlerts()
		]);

		if (results[0].status === 'fulfilled') {
			const net = results[0].value;
			trustedProxiesText = net.trusted_proxies.join('\n');
			realIpHeader = net.real_ip_header;
			extraSansText = net.extra_sans.join('\n');
			httpsAddr = net.https_addr;
		}
		if (results[1].status === 'fulfilled') {
			tlsAlerts = results[1].value.alerts.filter(
				(a) => a.id === 'server_cert_old_ca' || a.id === 'server_cert_expiring'
			);
		}

		loading = false;
	}

	// --- Network Settings ---
	async function saveNetworkSettings() {
		clearError();
		try {
			const proxies = trustedProxiesText
				.split('\n')
				.map((s) => s.trim())
				.filter((s) => s.length > 0);
			const sans = extraSansText
				.split('\n')
				.map((s) => s.trim())
				.filter((s) => s.length > 0);
			const res = await updateNetworkSettings({
				trusted_proxies: proxies,
				real_ip_header: realIpHeader,
				extra_sans: sans,
				https_addr: httpsAddr
			});
			trustedProxiesText = res.trusted_proxies.join('\n');
			realIpHeader = res.real_ip_header;
			extraSansText = res.extra_sans.join('\n');
			httpsAddr = res.https_addr;
			showSuccess('Network settings saved.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save network settings');
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

{#if $user && canManageGlobalSettings}
	<h1 class="h1 mb-6">Global Settings</h1>

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
			<p>Loading global settings...</p>
		</div>
	{:else}
		<!-- Section 1: Network Settings -->
		<div class="card mb-6 p-6">
			<h2 class="h3 mb-4">Network Settings</h2>
			<p class="mb-4 text-surface-600 dark:text-surface-400">
				Configure reverse proxy trust, client IP detection, and listen addresses.
				Changes to listen addresses require a restart to take effect.
			</p>

			<label class="label mb-4">
				<span>Trusted Proxies (one IP/CIDR per line)</span>
				<textarea
					class="textarea"
					rows="3"
					placeholder="e.g. 10.0.0.0/8&#10;192.168.1.1"
					bind:value={trustedProxiesText}
				></textarea>
			</label>

			<label class="label mb-4">
				<span>Real IP Header</span>
				<select class="select" bind:value={realIpHeader}>
					<option value="X-Forwarded-For">X-Forwarded-For</option>
					<option value="Forwarded">Forwarded (RFC 7239)</option>
					<option value="X-Real-Ip">X-Real-Ip</option>
					<option value="CF-Connecting-IP">CF-Connecting-IP</option>
					<option value="True-Client-IP">True-Client-IP</option>
				</select>
			</label>

			<label class="label mb-4">
				<span>Extra SANs (one IP or DNS name per line)</span>
				<textarea
					class="textarea"
					rows="3"
					placeholder="e.g. controller.local&#10;192.168.1.100"
					bind:value={extraSansText}
				></textarea>
			</label>

			<label class="label mb-4">
				<span>HTTPS Listen Address <span class="badge preset-tonal-warning ml-2 text-xs">Requires restart</span></span>
				<input class="input" type="text" bind:value={httpsAddr} />
			</label>

			<button class="btn preset-filled-primary-500" onclick={saveNetworkSettings}>
				Save
			</button>
		</div>

		<!-- Section 2: Controller TLS Certificate -->
		<div class="card mb-6 p-6">
			<h2 class="h3 mb-4">Controller TLS Certificate</h2>
			<p class="mb-4 text-surface-600 dark:text-surface-400">
				The controller's HTTPS certificate is automatically renewed before expiration.
				You can manually renew it here to re-issue under the current active CA.
			</p>

			{#if tlsAlerts.length > 0}
				{#each tlsAlerts as alert (alert.id)}
					<aside class="mb-4 rounded-lg p-4 {alert.severity === 'warning' ? 'preset-filled-warning-500' : 'preset-filled-surface-400-600'}">
						<p>{alert.message}</p>
					</aside>
				{/each}
			{/if}

			<button
				class="btn preset-filled-primary-500"
				onclick={handleRenewServerCert}
				disabled={renewingCert}
			>
				{renewingCert ? 'Renewing...' : 'Renew Server Certificate'}
			</button>
		</div>
	{/if}
{/if}
