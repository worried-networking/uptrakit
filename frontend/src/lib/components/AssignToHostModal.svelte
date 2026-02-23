<script lang="ts">
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import {
		getSoftwareItem,
		getHosts,
		assignHostsToSoftwareItem,
		unassignHostFromSoftwareItem,
		getProviderConfigs
	} from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { HostResponse, ProviderConfigResponse } from '$lib/types';

	let {
		softwareItemId,
		softwareItemName,
		onclose,
		onsuccess
	}: {
		softwareItemId: string;
		softwareItemName: string;
		onclose: () => void;
		onsuccess: () => void;
	} = $props();

	let allHosts: HostResponse[] = $state([]);
	const originalAssignedIds = new SvelteSet<string>();
	const selectedIds = new SvelteSet<string>();
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let submitting: boolean = $state(false);

	let providerConfigs: ProviderConfigResponse[] = $state([]);
	let selectedProviderConfigId: string = $state('');
	let packageIdentifier: string = $state('');
	let configsLoading: boolean = $state(true);

	const toAdd = $derived([...selectedIds].filter((id) => !originalAssignedIds.has(id)));

	onMount(async () => {
		try {
			const [detail, hostsResult, configsResult] = await Promise.all([
				getSoftwareItem(softwareItemId),
				getHosts(1, 200),
				getProviderConfigs(1, 500)
			]);
			allHosts = hostsResult.items;
			for (const h of detail.hosts) {
				originalAssignedIds.add(h.host_id);
				selectedIds.add(h.host_id);
			}
			providerConfigs = configsResult.items;
			selectedProviderConfigId = configsResult.items[0]?.id ?? '';
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load data.';
		} finally {
			loading = false;
			configsLoading = false;
		}
	});

	function toggleHost(hostId: string) {
		if (selectedIds.has(hostId)) {
			selectedIds.delete(hostId);
		} else {
			selectedIds.add(hostId);
		}
	}

	async function submit() {
		if (submitting) return;

		const toRemove = [...originalAssignedIds].filter((id) => !selectedIds.has(id));

		if (toAdd.length === 0 && toRemove.length === 0) {
			onclose();
			return;
		}

		submitting = true;
		try {
			const tasks: Promise<unknown>[] = [];
			if (toAdd.length > 0) {
				tasks.push(
					assignHostsToSoftwareItem(softwareItemId, {
						host_assignments: toAdd.map((host_id) => ({
							host_id,
							provider_config_id: selectedProviderConfigId || undefined,
							package_identifier: packageIdentifier.trim() || undefined
						}))
					})
				);
			}
			for (const hostId of toRemove) {
				tasks.push(unassignHostFromSoftwareItem(softwareItemId, hostId));
			}
			await Promise.all(tasks);
			showSuccess('Host assignments updated.');
			onsuccess();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update host assignments.');
		} finally {
			submitting = false;
		}
	}
</script>

<ModalBackdrop {onclose}>
	<div
		class="card bg-surface-50 dark:bg-surface-900 w-full max-w-lg max-h-[80vh] flex flex-col space-y-4 p-6 shadow-xl"
		role="dialog"
		aria-modal="true"
		aria-labelledby="assign-host-title"
	>
		<div>
			<h3 class="h3" id="assign-host-title">Assign to Hosts</h3>
			<p class="text-sm text-surface-500">
				Select hosts to track <strong>{softwareItemName}</strong> on.
			</p>
		</div>

		{#if loading}
			<p class="text-surface-500">Loading...</p>
		{:else if loadError}
			<aside class="rounded-lg p-4 preset-filled-error-500">
				<p>{loadError}</p>
			</aside>
		{:else if allHosts.length === 0}
			<aside class="rounded-lg p-4 preset-tonal-surface">
				<p class="text-sm">No hosts are registered yet. Hosts appear once an approved agent reports from a machine.</p>
			</aside>
		{:else}
			<ul class="overflow-y-auto space-y-1 flex-1 min-h-0">
				{#each allHosts as host (host.id)}
					<li>
						<label
							class="flex items-center gap-3 rounded-md px-3 py-2 cursor-pointer hover:bg-surface-100 dark:hover:bg-surface-800"
						>
							<input
								class="checkbox"
								type="checkbox"
								checked={selectedIds.has(host.id)}
								onchange={() => toggleHost(host.id)}
							/>
							<span class="flex-1 min-w-0">
								<span class="block font-medium truncate">{host.friendly_name}</span>
								<span class="block text-xs text-surface-500 truncate">{host.hostname}</span>
							</span>
						</label>
					</li>
				{/each}
			</ul>

			{#if toAdd.length > 0}
				<div class="space-y-3 border-t border-surface-200 dark:border-surface-700 pt-3">
					<p class="text-sm font-medium">Provider config for new assignments</p>
					{#if configsLoading}
						<p class="text-sm text-surface-500">Loading provider configs...</p>
					{:else}
						<label class="label">
							<span>Provider config</span>
							<select class="select" bind:value={selectedProviderConfigId}>
								<option value="">None</option>
								{#each providerConfigs as config (config.id)}
									<option value={config.id}>{config.name} ({config.provider_type})</option>
								{/each}
							</select>
						</label>
						<label class="label">
							<span>Package identifier <span class="text-surface-400">(optional)</span></span>
							<input class="input" bind:value={packageIdentifier} placeholder="e.g. firefox" />
						</label>
					{/if}
				</div>
			{/if}
		{/if}

		<div class="flex justify-end gap-2">
			<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
			<button class="btn preset-filled-primary-500" disabled={submitting || loading || !!loadError} onclick={submit}>
				{submitting ? 'Saving...' : 'Save'}
			</button>
		</div>
	</div>
</ModalBackdrop>
