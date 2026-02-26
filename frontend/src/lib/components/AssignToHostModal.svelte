<script lang="ts">
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import {
		getSoftwareItem,
		getHosts,
		assignHostsToSoftwareItem,
		unassignHostFromSoftwareItem,
		getPluginConfigs
	} from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { HostResponse, PluginConfigResponse, HostPluginRoleAssignment } from '$lib/types';

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

	type RoleKey = 'detect_version' | 'fetch_releases' | 'execute_update';

	const ROLE_LABELS: Record<RoleKey, string> = {
		detect_version: 'Detect Version',
		fetch_releases: 'Fetch Releases',
		execute_update: 'Execute Update'
	};

	const ALL_ROLES: RoleKey[] = ['detect_version', 'fetch_releases', 'execute_update'];

	let allHosts: HostResponse[] = $state([]);
	const originalAssignedIds = new SvelteSet<string>();
	const selectedIds = new SvelteSet<string>();
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let submitting: boolean = $state(false);

	let pluginConfigs: PluginConfigResponse[] = $state([]);

	let roleAssignments: Record<RoleKey, { enabled: boolean; plugin_config_id: string; package_identifier: string }> =
		$state({
			detect_version: { enabled: true, plugin_config_id: '', package_identifier: '' },
			fetch_releases: { enabled: false, plugin_config_id: '', package_identifier: '' },
			execute_update: { enabled: false, plugin_config_id: '', package_identifier: '' }
		});

	const toAdd = $derived([...selectedIds].filter((id) => !originalAssignedIds.has(id)));

	onMount(async () => {
		try {
			const [detail, hostsResult, configsResult] = await Promise.all([
				getSoftwareItem(softwareItemId),
				getHosts(1, 200),
				getPluginConfigs(1, 500)
			]);
			allHosts = hostsResult.items;
			for (const h of detail.hosts) {
				originalAssignedIds.add(h.host_id);
				selectedIds.add(h.host_id);
			}
			pluginConfigs = configsResult.items;
			const firstId = configsResult.items[0]?.id ?? '';
			roleAssignments = {
				detect_version: { enabled: true, plugin_config_id: firstId, package_identifier: '' },
				fetch_releases: { enabled: false, plugin_config_id: firstId, package_identifier: '' },
				execute_update: { enabled: false, plugin_config_id: firstId, package_identifier: '' }
			};
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load data.';
		} finally {
			loading = false;
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
				const plugins: HostPluginRoleAssignment[] = ALL_ROLES.filter(
					(role) => roleAssignments[role].enabled && roleAssignments[role].plugin_config_id
				).map((role) => ({
					role,
					plugin_config_id: roleAssignments[role].plugin_config_id || undefined,
					package_identifier: roleAssignments[role].package_identifier.trim() || undefined
				}));

				tasks.push(
					assignHostsToSoftwareItem(softwareItemId, {
						host_assignments: toAdd.map((host_id) => ({ host_id, plugins }))
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
		class="card bg-surface-50 dark:bg-surface-900 w-full max-w-2xl max-h-[85vh] flex flex-col space-y-4 p-6 shadow-xl"
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
					<p class="text-sm font-medium">Role assignments for new hosts</p>
					<div class="table-wrap">
						<table class="table text-sm">
							<thead>
								<tr>
									<th class="w-36">Role</th>
									<th>Plugin Config</th>
									<th>Package Identifier</th>
								</tr>
							</thead>
							<tbody>
								{#each ALL_ROLES as role (role)}
									{@const a = roleAssignments[role]}
									<tr>
										<td>
											<label class="flex items-center gap-2 cursor-pointer">
												<input class="checkbox" type="checkbox" bind:checked={roleAssignments[role].enabled} />
												<span class="whitespace-nowrap">{ROLE_LABELS[role]}</span>
											</label>
										</td>
										<td>
											<select
												class="select text-sm"
												bind:value={roleAssignments[role].plugin_config_id}
												disabled={!a.enabled}
											>
												<option value="">— none —</option>
												{#each pluginConfigs as config (config.id)}
													<option value={config.id}>{config.name}</option>
												{/each}
											</select>
										</td>
										<td>
											<input
												class="input text-sm"
												type="text"
												placeholder="e.g. owner/repo"
												bind:value={roleAssignments[role].package_identifier}
												disabled={!a.enabled}
											/>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
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
