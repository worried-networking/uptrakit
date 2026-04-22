<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import Modal from '$lib/components/Modal.svelte';
	import Button from '$lib/components/Button.svelte';
	import { Callout } from '$lib/components/ui';
	import CheckboxList from '$lib/components/CheckboxList.svelte';
	import type { CheckboxListItem } from '$lib/components/CheckboxList.svelte';
	import {
		getSoftwareItem,
		getHosts,
		assignHostsToSoftwareItem,
		unassignHostFromSoftwareItem,
		getPluginConfigs,
		listPluginTypes
	} from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { HostResponse, PluginConfigResponse, HostPluginRoleAssignment, PluginTypeInfo } from '$lib/types';

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

	type StandardRoleKey = 'detect_version' | 'fetch_releases' | 'execute_update';
	type HookRoleKey = 'pre_update_hook' | 'post_update_hook';

	const ROLE_LABELS: Record<StandardRoleKey | HookRoleKey, string> = {
		detect_version: 'Detect Version',
		fetch_releases: 'Fetch Releases',
		execute_update: 'Execute Update',
		pre_update_hook: 'Pre-Update Hooks',
		post_update_hook: 'Post-Update Hooks'
	};

	const STANDARD_ROLES: StandardRoleKey[] = ['detect_version', 'fetch_releases', 'execute_update'];
	const HOOK_ROLES: HookRoleKey[] = ['pre_update_hook', 'post_update_hook'];

	interface HookEntry {
		localKey: number;
		plugin_config_id: string;
	}

	let allHosts: HostResponse[] = $state([]);
	const originalAssignedIds = new SvelteSet<string>();
	const selectedIds = new SvelteSet<string>();
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let submitting: boolean = $state(false);
	let assignmentError: string | null = $state(null);
	let standardRoleErrors: Partial<Record<StandardRoleKey, string>> = $state({});
	let hookEntryErrors: Record<number, string> = $state({});

	let pluginConfigs: PluginConfigResponse[] = $state([]);
	let pluginTypes: PluginTypeInfo[] = $state([]);

	function isHookPluginType(pluginType: string): boolean {
		const pt = pluginTypes.find((t) => t.plugin_type === pluginType);
		if (!pt) return false;
		return pt.capabilities.some((c) => c === 'update_lifecycle');
	}

	function hookConfigsForRole(_role: HookRoleKey): PluginConfigResponse[] {
		return pluginConfigs.filter((c) => isHookPluginType(c.plugin_type));
	}

	function standardConfigsForRole(_role: StandardRoleKey): PluginConfigResponse[] {
		return pluginConfigs.filter((c) => !isHookPluginType(c.plugin_type));
	}

	let standardAssignments: Record<
		StandardRoleKey,
		{ enabled: boolean; plugin_config_id: string; package_identifier: string; execution_site: string }
	> = $state({
		detect_version: { enabled: true, plugin_config_id: '', package_identifier: '', execution_site: 'auto' },
		fetch_releases: { enabled: false, plugin_config_id: '', package_identifier: '', execution_site: 'auto' },
		execute_update: { enabled: false, plugin_config_id: '', package_identifier: '', execution_site: 'auto' }
	});

	let hookLists: Record<HookRoleKey, HookEntry[]> = $state({
		pre_update_hook: [],
		post_update_hook: []
	});

	let nextKey = 0;

	function addHook(role: HookRoleKey) {
		hookLists[role] = [...hookLists[role], { localKey: nextKey++, plugin_config_id: '' }];
		assignmentError = null;
	}

	function removeHook(role: HookRoleKey, localKey: number) {
		hookLists[role] = hookLists[role].filter((e) => e.localKey !== localKey);
		const next = { ...hookEntryErrors };
		delete next[localKey];
		hookEntryErrors = next;
		assignmentError = null;
	}

	function clearStandardRoleError(role: StandardRoleKey) {
		if (!standardRoleErrors[role]) return;
		standardRoleErrors = { ...standardRoleErrors, [role]: undefined };
	}

	function clearHookEntryError(localKey: number) {
		if (!hookEntryErrors[localKey]) return;
		const next = { ...hookEntryErrors };
		delete next[localKey];
		hookEntryErrors = next;
	}

	function validateAssignmentsForNewHosts(): boolean {
		const nextStandardRoleErrors: Partial<Record<StandardRoleKey, string>> = {};
		const nextHookEntryErrors: Record<number, string> = {};

		for (const role of STANDARD_ROLES) {
			const assignment = standardAssignments[role];
			if (!assignment.enabled) continue;
			if (!assignment.plugin_config_id) {
				nextStandardRoleErrors[role] = `Select a plugin config for ${ROLE_LABELS[role]}.`;
			}
		}

		for (const role of HOOK_ROLES) {
			for (const entry of hookLists[role]) {
				if (!entry.plugin_config_id) {
					nextHookEntryErrors[entry.localKey] = `Select a plugin config for ${ROLE_LABELS[role]}.`;
				}
			}
		}

		standardRoleErrors = nextStandardRoleErrors;
		hookEntryErrors = nextHookEntryErrors;

		const hasErrors = Object.keys(nextStandardRoleErrors).length > 0 || Object.keys(nextHookEntryErrors).length > 0;
		assignmentError = hasErrors ? 'Resolve the highlighted role assignment errors before saving.' : null;
		return !hasErrors;
	}

	const toAdd = $derived([...selectedIds].filter((id) => !originalAssignedIds.has(id)));

	onMount(async () => {
		try {
			const [detail, hostsResult, configsResult, typesResult] = await Promise.all([
				getSoftwareItem(softwareItemId),
				getHosts(1, 200),
				getPluginConfigs(1, 500),
				listPluginTypes()
			]);
			allHosts = hostsResult.items;
			for (const h of detail.hosts) {
				originalAssignedIds.add(h.host_id);
				selectedIds.add(h.host_id);
			}
			pluginConfigs = configsResult.items;
			pluginTypes = typesResult;
			const firstStandardId = configsResult.items.find((c) => !isHookPluginType(c.plugin_type))?.id ?? '';
			standardAssignments = {
				detect_version: {
					enabled: true,
					plugin_config_id: firstStandardId,
					package_identifier: '',
					execution_site: 'auto'
				},
				fetch_releases: {
					enabled: false,
					plugin_config_id: firstStandardId,
					package_identifier: '',
					execution_site: 'auto'
				},
				execute_update: {
					enabled: false,
					plugin_config_id: firstStandardId,
					package_identifier: '',
					execution_site: 'auto'
				}
			};
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load data.';
		} finally {
			loading = false;
		}
	});

	const hostItems = $derived<CheckboxListItem[]>(
		allHosts.map((h) => ({ value: h.id, label: h.friendly_name, sublabel: h.hostname }))
	);

	async function submit() {
		if (submitting) return;
		assignmentError = null;
		submitting = true;
		await tick();

		const pendingAdd = [...selectedIds].filter((id) => !originalAssignedIds.has(id));
		const toRemove = [...originalAssignedIds].filter((id) => !selectedIds.has(id));

		if (pendingAdd.length === 0 && toRemove.length === 0) {
			submitting = false;
			onclose();
			return;
		}
		if (pendingAdd.length > 0 && !validateAssignmentsForNewHosts()) {
			submitting = false;
			return;
		}

		try {
			const tasks: Promise<unknown>[] = [];
			if (pendingAdd.length > 0) {
				const plugins: HostPluginRoleAssignment[] = [];

				for (const role of STANDARD_ROLES) {
					const a = standardAssignments[role];
					if (a.enabled && a.plugin_config_id) {
						plugins.push({
							role,
							plugin_config_id: a.plugin_config_id,
							package_identifier: a.package_identifier.trim() || undefined,
							execution_site: a.execution_site !== 'auto' ? a.execution_site : undefined
						});
					}
				}

				for (const role of HOOK_ROLES) {
					hookLists[role].forEach((entry, idx) => {
						if (entry.plugin_config_id) {
							plugins.push({ role, ordinal: idx, plugin_config_id: entry.plugin_config_id });
						}
					});
				}

				tasks.push(
					assignHostsToSoftwareItem(softwareItemId, {
						host_assignments: pendingAdd.map((host_id) => ({ host_id, plugins }))
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

<Modal title="Assign to Hosts" {onclose} maxWidth="max-w-2xl max-h-[85vh] flex flex-col">
	<p class="text-sm text-surface-500">
		Select hosts to track <strong>{softwareItemName}</strong> on.
	</p>
	{#if assignmentError}
		<Callout tone="danger" message={assignmentError} />
	{/if}

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
		<CheckboxList items={hostItems} selected={selectedIds} maxHeight="max-h-64" />

		{#if toAdd.length > 0}
			<div class="space-y-4 border-t border-surface-200 dark:border-surface-700 pt-3">
				<p class="text-sm font-medium">Role assignments for new hosts</p>

				<!-- detect_version + fetch_releases table -->
				<div class="table-wrap">
					<table class="table text-sm">
						<thead>
							<tr>
								<th class="w-36">Role</th>
								<th>Plugin Config</th>
								<th>Package ID</th>
								<th class="w-36">Execution Site</th>
							</tr>
						</thead>
						<tbody>
							{#each ['detect_version', 'fetch_releases'] as const as role (role)}
								{@const a = standardAssignments[role]}
								<tr>
									<td>
										<label class="flex items-center gap-2 cursor-pointer">
											<input
												class="checkbox"
												type="checkbox"
												bind:checked={standardAssignments[role].enabled}
												onchange={() => {
													if (!standardAssignments[role].enabled) {
														standardAssignments[role].plugin_config_id = '';
													}
													clearStandardRoleError(role);
													assignmentError = null;
												}}
											/>
											<span class="whitespace-nowrap">{ROLE_LABELS[role]}</span>
										</label>
									</td>
									<td>
										<select
											class="select text-sm"
											bind:value={standardAssignments[role].plugin_config_id}
											disabled={!a.enabled}
											aria-invalid={standardRoleErrors[role] ? 'true' : 'false'}
											onchange={() => {
												clearStandardRoleError(role);
												assignmentError = null;
											}}
										>
											<option value="">— none —</option>
											{#each standardConfigsForRole(role) as cfg (cfg.id)}
												<option value={cfg.id}>{cfg.name}</option>
											{/each}
										</select>
										{#if standardRoleErrors[role]}
											<p class="mt-1 text-xs text-[var(--color-error)]">{standardRoleErrors[role]}</p>
										{/if}
									</td>
									<td>
										<input
											class="input text-sm"
											type="text"
											placeholder="e.g. owner/repo"
											bind:value={standardAssignments[role].package_identifier}
											disabled={!a.enabled}
										/>
									</td>
									<td>
										<select
											class="select text-sm"
											bind:value={standardAssignments[role].execution_site}
											disabled={!a.enabled}
										>
											<option value="auto">Auto</option>
											<option value="agent">Agent</option>
											{#if role === 'fetch_releases'}
												<option value="controller">Controller</option>
											{/if}
										</select>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>

				<!-- Pre-Update Hooks -->
				{#each ['pre_update_hook'] as const as hookRole (hookRole)}
					{@const entries = hookLists[hookRole]}
					{@const hookConfigs = hookConfigsForRole(hookRole)}
					<div class="space-y-2">
						<div class="flex items-center justify-between">
							<span class="text-sm font-medium">{ROLE_LABELS[hookRole]}</span>
							<Button variant="secondary" size="sm" type="button" onclick={() => addHook(hookRole)}>+ Add</Button>
						</div>
						{#if entries.length === 0}
							<p class="text-xs text-surface-400">No pre-update hooks configured.</p>
						{:else}
							<div class="space-y-2">
								{#each entries as entry (entry.localKey)}
									<div class="space-y-1">
										<div class="flex items-center gap-2">
											<select
												class="select text-sm flex-1"
												bind:value={entry.plugin_config_id}
												aria-invalid={hookEntryErrors[entry.localKey] ? 'true' : 'false'}
												onchange={() => {
													clearHookEntryError(entry.localKey);
													assignmentError = null;
												}}
											>
												<option value="">— select plugin —</option>
												{#each hookConfigs as cfg (cfg.id)}
													<option value={cfg.id}>{cfg.name}</option>
												{/each}
											</select>
											<Button
												variant="danger"
												size="sm"
												class="shrink-0"
												type="button"
												onclick={() => removeHook(hookRole, entry.localKey)}>Remove</Button
											>
										</div>
										{#if hookEntryErrors[entry.localKey]}
											<p class="text-xs text-[var(--color-error)]">{hookEntryErrors[entry.localKey]}</p>
										{/if}
									</div>
								{/each}
							</div>
						{/if}
					</div>
				{/each}

				<!-- execute_update table -->
				<div class="table-wrap">
					<table class="table text-sm">
						<thead>
							<tr>
								<th class="w-36">Role</th>
								<th>Plugin Config</th>
								<th>Package ID</th>
								<th class="w-36">Execution Site</th>
							</tr>
						</thead>
						<tbody>
							{#each ['execute_update'] as const as role (role)}
								{@const a = standardAssignments[role]}
								<tr>
									<td>
										<label class="flex items-center gap-2 cursor-pointer">
											<input
												class="checkbox"
												type="checkbox"
												bind:checked={standardAssignments[role].enabled}
												onchange={() => {
													if (!standardAssignments[role].enabled) {
														standardAssignments[role].plugin_config_id = '';
													}
													clearStandardRoleError(role);
													assignmentError = null;
												}}
											/>
											<span class="whitespace-nowrap">{ROLE_LABELS[role]}</span>
										</label>
									</td>
									<td>
										<select
											class="select text-sm"
											bind:value={standardAssignments[role].plugin_config_id}
											disabled={!a.enabled}
											aria-invalid={standardRoleErrors[role] ? 'true' : 'false'}
											onchange={() => {
												clearStandardRoleError(role);
												assignmentError = null;
											}}
										>
											<option value="">— none —</option>
											{#each standardConfigsForRole(role) as cfg (cfg.id)}
												<option value={cfg.id}>{cfg.name}</option>
											{/each}
										</select>
										{#if standardRoleErrors[role]}
											<p class="mt-1 text-xs text-[var(--color-error)]">{standardRoleErrors[role]}</p>
										{/if}
									</td>
									<td>
										<input
											class="input text-sm"
											type="text"
											placeholder="e.g. owner/repo"
											bind:value={standardAssignments[role].package_identifier}
											disabled={!a.enabled}
										/>
									</td>
									<td>
										<select
											class="select text-sm"
											bind:value={standardAssignments[role].execution_site}
											disabled={!a.enabled}
										>
											<option value="auto">Auto</option>
											<option value="agent">Agent</option>
										</select>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>

				<!-- Post-Update Hooks -->
				{#each ['post_update_hook'] as const as hookRole (hookRole)}
					{@const entries = hookLists[hookRole]}
					{@const hookConfigs = hookConfigsForRole(hookRole)}
					<div class="space-y-2">
						<div class="flex items-center justify-between">
							<span class="text-sm font-medium">{ROLE_LABELS[hookRole]}</span>
							<Button variant="secondary" size="sm" type="button" onclick={() => addHook(hookRole)}>+ Add</Button>
						</div>
						{#if entries.length === 0}
							<p class="text-xs text-surface-400">No post-update hooks configured.</p>
						{:else}
							<div class="space-y-2">
								{#each entries as entry (entry.localKey)}
									<div class="space-y-1">
										<div class="flex items-center gap-2">
											<select
												class="select text-sm flex-1"
												bind:value={entry.plugin_config_id}
												aria-invalid={hookEntryErrors[entry.localKey] ? 'true' : 'false'}
												onchange={() => {
													clearHookEntryError(entry.localKey);
													assignmentError = null;
												}}
											>
												<option value="">— select plugin —</option>
												{#each hookConfigs as cfg (cfg.id)}
													<option value={cfg.id}>{cfg.name}</option>
												{/each}
											</select>
											<Button
												variant="danger"
												size="sm"
												class="shrink-0"
												type="button"
												onclick={() => removeHook(hookRole, entry.localKey)}>Remove</Button
											>
										</div>
										{#if hookEntryErrors[entry.localKey]}
											<p class="text-xs text-[var(--color-error)]">{hookEntryErrors[entry.localKey]}</p>
										{/if}
									</div>
								{/each}
							</div>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	{/if}

	{#snippet footer()}
		<Button variant="secondary" onclick={onclose}>Cancel</Button>
		<Button variant="primary" loading={submitting} disabled={loading || !!loadError} onclick={submit}>Save</Button>
	{/snippet}
</Modal>
