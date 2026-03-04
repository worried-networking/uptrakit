<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { page } from '$app/stores';
	import { getUser } from '$lib/auth.svelte';
	import {
		getSoftwareItem,
		checkSoftwareItemVersions,
		checkSoftwareItemVersionsHost,
		triggerSoftwareUpdate
	} from '$lib/api';
	import { formatDate, formatVersion } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import TerminalOutput from '$lib/components/TerminalOutput.svelte';
	import { connectOutputStream } from '$lib/sse';
	import type { SseConnectionState } from '$lib/sse';
	import { Permission } from '$lib/types';
	import type { AttestationStatus, SoftwareItemDetailResponse, SoftwareItemHostSummary } from '$lib/types';

	const id = $derived($page.params.id as string);

	let item: SoftwareItemDetailResponse | null = $state(null);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);
	let checkingAll: boolean = $state(false);
	let checkingHostId: string | null = $state(null);

	// Update confirm modal state
	let updateModal: { host: SoftwareItemHostSummary; toVersion: string } | null = $state(null);
	let updateTriggering: boolean = $state(false);

	// Release notes modal state
	interface ReleaseMeta {
		release_url?: string;
		release_notes?: string;
		tag?: string;
		published_at?: string;
		attestation_status?: AttestationStatus;
	}
	let releaseNotesModal: {
		softwareName: string;
		hostName: string;
		meta: ReleaseMeta;
	} | null = $state(null);

	// Live terminal modal state
	let liveModal: { updateHistoryId: string; hostName: string } | null = $state(null);
	let liveStreamState: SseConnectionState = $state('disconnected');
	let liveDisconnect: (() => void) | null = null;
	let liveTerminalRef: TerminalOutput | undefined = $state(undefined);

	const canView = $derived(getUser()?.permissions.includes(Permission.ViewSoftware) ?? false);
	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	let refreshInterval: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		if (canView) {
			loadItem();
			refreshInterval = setInterval(() => {
				if (document.visibilityState === 'visible') loadItem(true);
			}, 30_000);
		}
	});

	onDestroy(() => {
		if (refreshInterval) clearInterval(refreshInterval);
		closeLiveModal();
	});

	async function loadItem(background = false) {
		if (!background) {
			loading = true;
			error = null;
		}
		try {
			item = await getSoftwareItem(id);
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load software item';
			}
		} finally {
			if (!background) loading = false;
		}
	}

	async function checkAllVersions() {
		if (!item || checkingAll) return;
		checkingAll = true;
		try {
			const result = await checkSoftwareItemVersions(item.id);
			if (result.agents_notified > 0) {
				showSuccess(`Version check triggered — ${result.agents_notified} agent(s) notified`);
			} else {
				showSuccess('Version check queued — no agents currently connected');
			}
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger version check');
		} finally {
			checkingAll = false;
		}
	}

	async function checkHostVersions(hostId: string) {
		if (!item || checkingHostId) return;
		checkingHostId = hostId;
		try {
			const result = await checkSoftwareItemVersionsHost(item.id, hostId);
			if (result.agents_notified > 0) {
				showSuccess('Version check triggered for host');
			} else {
				showSuccess('Version check queued — no agents currently connected');
			}
			// Refresh data after a short delay to pick up results.
			setTimeout(() => loadItem(true), 2000);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger version check');
		} finally {
			checkingHostId = null;
		}
	}

	function openUpdateModal(host: SoftwareItemHostSummary) {
		const toVersion = host.latest_version ?? item?.latest_version ?? '';
		updateModal = { host, toVersion };
	}

	async function executeUpdate() {
		if (!item || !updateModal || updateTriggering) return;
		updateTriggering = true;
		try {
			const hostName = updateModal.host.hostname;
			const res = await triggerSoftwareUpdate(item.id, updateModal.host.host_id, {
				to_version: updateModal.toVersion
			});
			showSuccess(`Update triggered — history ID: ${res.update_history_id}`);
			updateModal = null;
			// Open the live terminal modal
			openLiveModal(res.update_history_id, hostName);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger update');
		} finally {
			updateTriggering = false;
		}
	}

	function openLiveModal(updateHistoryId: string, hostName: string) {
		liveModal = { updateHistoryId, hostName };
		liveStreamState = 'connecting';
		// Defer SSE connection to next tick so the terminal has mounted
		setTimeout(() => {
			liveDisconnect = connectOutputStream(updateHistoryId, {
				onOutput: (line) => {
					if (liveTerminalRef) {
						liveTerminalRef.write(line.text);
					}
				},
				onCompleted: () => {
					// Refresh host data when update finishes
					loadItem(true);
				},
				onStateChange: (state) => {
					liveStreamState = state;
				},
				onError: (err) => {
					showError(`Stream error: ${err}`);
				}
			});
		}, 0);
	}

	function closeLiveModal() {
		if (liveDisconnect) {
			liveDisconnect();
			liveDisconnect = null;
		}
		liveModal = null;
		liveStreamState = 'disconnected';
	}

	function getReleaseMeta(host: SoftwareItemHostSummary): ReleaseMeta | null {
		const meta = host.latest_release_metadata;
		if (!meta) return null;
		const knownStatuses: AttestationStatus[] = ['Verified', 'NotFound', 'Unverified'];
		const rawStatus = meta.attestation_status;
		const attestation_status: AttestationStatus | undefined =
			typeof rawStatus === 'string' && knownStatuses.includes(rawStatus as AttestationStatus)
				? (rawStatus as AttestationStatus)
				: undefined;
		return {
			release_url: typeof meta.release_url === 'string' ? meta.release_url : undefined,
			release_notes: typeof meta.release_notes === 'string' ? meta.release_notes : undefined,
			tag: typeof meta.tag === 'string' ? meta.tag : undefined,
			published_at: typeof meta.published_at === 'string' ? meta.published_at : undefined,
			attestation_status
		};
	}

	function openReleaseNotesModal(host: SoftwareItemHostSummary) {
		const meta = getReleaseMeta(host);
		if (!meta) return;
		releaseNotesModal = { softwareName: item?.name ?? '', hostName: host.hostname, meta };
	}

	function versionStatusLabel(host: SoftwareItemHostSummary): string {
		if (!host.installed_version) return 'Unknown';
		if (!host.latest_version) return 'Unknown latest';
		if (host.update_available) return 'Update Available';
		return 'Up-to-date';
	}

	function versionStatusClass(host: SoftwareItemHostSummary): string {
		if (!host.installed_version) return 'preset-tonal';
		if (host.update_available) return 'preset-filled-warning-500';
		return 'preset-filled-success-500';
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			if (liveModal) {
				closeLiveModal();
			} else if (releaseNotesModal) {
				releaseNotesModal = null;
			} else if (updateModal) {
				updateModal = null;
			}
		}
	}}
/>

{#if getUser()}
	<div class="mb-4">
		<a href="/software" class="text-sm text-surface-500 hover:underline">← Back to Software</a>
	</div>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view software items.</p>
		</aside>
	{:else if loading}
		<p>Loading...</p>
	{:else if error}
		<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
			<p>{error}</p>
			<button class="btn preset-filled-primary-500 mt-2" onclick={() => loadItem()}>Retry</button>
		</aside>
	{:else if item}
		<!-- Header -->
		<div class="mb-6 flex flex-wrap items-start justify-between gap-4">
			<div>
				<h1 class="h1">{item.name}</h1>
				<div class="mt-2 flex flex-wrap items-center gap-2">
					{#if item.enabled}
						<span class="badge preset-filled-success-500">Enabled</span>
					{:else}
						<span class="badge preset-tonal">Disabled</span>
					{/if}
					{#if item.update_available}
						<span class="badge preset-filled-warning-500">Update Available</span>
					{/if}
					{#if item.plugins.length > 0}
						<span class="text-sm text-surface-500">{item.plugins.join(', ')}</span>
					{/if}
				</div>
				<div class="mt-2 space-y-1 text-sm text-surface-500">
					{#if item.latest_version}
						<p>
							Latest version: <span
								class="font-medium text-surface-700 dark:text-surface-300"
								title={item.latest_version}>{formatVersion(item.latest_version)}</span
							>
						</p>
					{/if}
					<p>Last checked: {formatDate(item.last_checked_at)}</p>
					<p>{item.host_count} host{item.host_count !== 1 ? 's' : ''} assigned</p>
				</div>
			</div>
			{#if canManage}
				<button class="btn preset-tonal-surface" onclick={checkAllVersions} disabled={checkingAll}>
					{checkingAll ? 'Checking…' : 'Check All Versions'}
				</button>
			{/if}
		</div>

		<!-- Hosts table -->
		<div class="table-wrap">
			<table class="table">
				<thead>
					<tr>
						<th>Hostname</th>
						<th>Installed Version</th>
						<th>Latest Version</th>
						<th>Status</th>
						<th>Detected At</th>
						{#if canManage}
							<th>Actions</th>
						{/if}
					</tr>
				</thead>
				<tbody>
					{#if item.hosts.length === 0}
						<tr>
							<td colspan={canManage ? 6 : 5} class="py-8 text-center">
								<p class="text-lg font-medium">No hosts assigned</p>
								<p class="mt-1 text-sm text-surface-500">Assign hosts to this software item to start tracking.</p>
							</td>
						</tr>
					{:else}
						{#each item.hosts as host (host.host_id)}
							<tr>
								<td>
									<a href="/hosts/{host.host_id}" class="hover:underline font-medium">{host.hostname}</a>
									{#if host.friendly_name && host.friendly_name !== host.hostname}
										<span class="block text-xs text-surface-500">{host.friendly_name}</span>
									{/if}
								</td>
								<td title={host.installed_version ?? undefined}>{formatVersion(host.installed_version)}</td>
								<td>
									<span title={host.latest_version ?? item.latest_version ?? undefined}
										>{formatVersion(host.latest_version ?? item.latest_version)}</span
									>
									{#if getReleaseMeta(host)}
										<button
											class="btn btn-sm preset-tonal ml-1"
											title="View release notes"
											onclick={() => openReleaseNotesModal(host)}>Notes</button
										>
									{/if}
									{#if getReleaseMeta(host)?.attestation_status === 'Verified'}
										<span
											class="badge preset-filled-success-500 ml-1 text-xs"
											title="GitHub Actions attestation verified">Attested</span
										>
									{:else if getReleaseMeta(host)?.attestation_status === 'NotFound'}
										<span class="badge preset-filled-error-500 ml-1 text-xs" title="No GitHub Actions attestation found"
											>Not attested</span
										>
									{/if}
								</td>
								<td>
									<span class="badge {versionStatusClass(host)}">{versionStatusLabel(host)}</span>
								</td>
								<td>{formatDate(host.installed_version_detected_at)}</td>
								{#if canManage}
									<td class="space-x-2 whitespace-nowrap">
										{#if host.update_available || item.latest_version}
											{@const updateToVer = host.latest_version ?? item?.latest_version ?? null}
											<button
												class="btn btn-sm preset-filled-warning-500"
												title={updateToVer ?? undefined}
												onclick={() => openUpdateModal(host)}
											>
												Update to {formatVersion(updateToVer)}
											</button>
										{/if}
										<button
											class="btn btn-sm preset-tonal"
											onclick={() => checkHostVersions(host.host_id)}
											disabled={checkingHostId === host.host_id}
										>
											{checkingHostId === host.host_id ? 'Checking…' : 'Check Versions'}
										</button>
									</td>
								{/if}
							</tr>
						{/each}
					{/if}
				</tbody>
			</table>
		</div>
	{/if}
{/if}

{#if updateModal}
	<ModalBackdrop onclose={() => (updateModal = null)}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-md space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">Confirm Update</h3>

			<p class="text-sm">
				Update <strong>{item?.name}</strong> on <strong>{updateModal.host.hostname}</strong>?
			</p>
			<div class="grid grid-cols-2 gap-4 text-sm">
				<div>
					<p class="text-surface-500">From</p>
					<p class="font-medium" title={updateModal.host.installed_version ?? undefined}>
						{formatVersion(updateModal.host.installed_version, 'unknown')}
					</p>
				</div>
				<div>
					<p class="text-surface-500">To</p>
					<p class="font-medium" title={updateModal.toVersion}>{formatVersion(updateModal.toVersion)}</p>
				</div>
			</div>

			{#if updateModal}
				{@const meta = getReleaseMeta(updateModal.host)}
				{#if meta?.release_url}
					<p class="text-sm">
						<a
							href={meta.release_url}
							target="_blank"
							rel="noopener noreferrer"
							class="text-primary-500 hover:underline">View release page ↗</a
						>
					</p>
				{/if}
				{#if meta?.release_notes}
					<details class="text-sm">
						<summary class="cursor-pointer text-surface-500 hover:text-surface-700">Release notes</summary>
						<pre class="mt-2 max-h-48 overflow-y-auto whitespace-pre-wrap text-xs">{meta.release_notes}</pre>
					</details>
				{/if}
				{#if meta?.attestation_status === 'NotFound'}
					<aside class="rounded-lg p-3 preset-filled-warning-500 text-sm" role="alert">
						<strong>Warning:</strong> No GitHub Actions attestation was found for this release. The artifacts may not have
						been produced by the official GitHub Actions workflow.
					</aside>
				{:else if meta?.attestation_status === 'Verified'}
					<p class="text-sm text-success-600 dark:text-success-400">&#10003; GitHub Actions attestation verified</p>
				{/if}
			{/if}

			<div class="flex justify-end gap-2">
				<button class="btn preset-tonal-surface" onclick={() => (updateModal = null)}>Cancel</button>
				<button class="btn preset-filled-warning-500" onclick={executeUpdate} disabled={updateTriggering}>
					{updateTriggering ? 'Triggering…' : 'Trigger Update'}
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}

{#if releaseNotesModal}
	<ModalBackdrop onclose={() => (releaseNotesModal = null)}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-2xl space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<div class="flex items-start justify-between gap-4">
				<div>
					<h3 class="h3">{releaseNotesModal.softwareName}</h3>
					<p class="text-sm text-surface-500">
						{releaseNotesModal.meta.tag ?? ''} on {releaseNotesModal.hostName}
						{#if releaseNotesModal.meta.published_at}
							· {formatDate(releaseNotesModal.meta.published_at)}
						{/if}
					</p>
				</div>
				{#if releaseNotesModal.meta.release_url}
					<a
						href={releaseNotesModal.meta.release_url}
						target="_blank"
						rel="noopener noreferrer"
						class="btn btn-sm preset-tonal-surface shrink-0">View on GitHub ↗</a
					>
				{/if}
			</div>

			{#if releaseNotesModal.meta.release_notes}
				<div class="overflow-y-auto max-h-96">
					<pre class="whitespace-pre-wrap text-sm leading-relaxed">{releaseNotesModal.meta.release_notes}</pre>
				</div>
			{:else}
				<p class="text-surface-500 text-sm">No release notes available.</p>
			{/if}

			<div class="flex justify-end">
				<button class="btn preset-tonal-surface" onclick={() => (releaseNotesModal = null)}>Close</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}

{#if liveModal}
	<ModalBackdrop onclose={closeLiveModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-3xl space-y-4 p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<div class="flex items-center justify-between">
				<div class="flex items-center gap-2">
					<h3 class="h3">Update Output</h3>
					{#if liveStreamState === 'streaming'}
						<span class="badge preset-filled-success-500 text-xs animate-pulse">Live</span>
					{:else if liveStreamState === 'connecting'}
						<span class="badge preset-tonal text-xs">Connecting…</span>
					{:else if liveStreamState === 'completed'}
						<span class="badge preset-filled-success-500 text-xs">Completed</span>
					{:else if liveStreamState === 'error'}
						<span class="badge preset-filled-error-500 text-xs">Error</span>
					{/if}
				</div>
				<p class="text-sm text-surface-500">{liveModal.hostName}</p>
			</div>

			<TerminalOutput bind:this={liveTerminalRef} class="h-96" />

			<div class="flex justify-end">
				<button class="btn preset-tonal-surface" onclick={closeLiveModal}>
					{liveStreamState === 'streaming' || liveStreamState === 'connecting' ? 'Close (update continues)' : 'Close'}
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}
