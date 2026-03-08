<script lang="ts">
	import { onMount } from 'svelte';
	import Modal from './Modal.svelte';
	import { getPluginConfigs, updateHostAssignment } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type {
		HostPluginRoleSummary,
		PluginConfigResponse,
		SoftwareItemDetailResponse,
		UpdateHostAssignmentRequest
	} from '$lib/types';

	type RoleKey = 'detect_version' | 'fetch_releases' | 'execute_update';

	const ROLE_LABELS: Record<RoleKey, string> = {
		detect_version: 'Detect Version',
		fetch_releases: 'Fetch Releases',
		execute_update: 'Execute Update'
	};

	const ROLE_DESCRIPTIONS: Record<RoleKey, string> = {
		detect_version: 'Checks what version is currently installed on the host.',
		fetch_releases: 'Queries upstream for the latest available version.',
		execute_update: 'Performs the actual software update on the host.'
	};

	const ALL_ROLES: RoleKey[] = ['detect_version', 'fetch_releases', 'execute_update'];

	interface RoleState {
		plugin_config_id: string;
		package_identifier: string;
		execution_site: string;
		config_override_text: string;
		config_override_error: string | null;
	}

	let {
		softwareItemId,
		hostId,
		hostName,
		softwareItemName,
		existingPlugins,
		onclose,
		onsuccess
	}: {
		softwareItemId: string;
		hostId: string;
		hostName: string;
		softwareItemName: string;
		existingPlugins: HostPluginRoleSummary[];
		onclose: () => void;
		onsuccess: (result: SoftwareItemDetailResponse) => void;
	} = $props();

	function makeInitialState(): Record<RoleKey, RoleState> {
		const result = {} as Record<RoleKey, RoleState>;
		for (const role of ALL_ROLES) {
			const existing = existingPlugins.find((p) => p.role === role);
			result[role] = existing
				? {
						plugin_config_id: existing.plugin_config_id,
						package_identifier: existing.package_identifier,
						execution_site: existing.execution_site || 'auto',
						config_override_text: existing.config_override ? JSON.stringify(existing.config_override, null, 2) : '',
						config_override_error: null
					}
				: {
						plugin_config_id: '',
						package_identifier: '',
						execution_site: 'auto',
						config_override_text: '',
						config_override_error: null
					};
		}
		return result;
	}

	let roleStates: Record<RoleKey, RoleState> = $state(makeInitialState());
	let pluginConfigs: PluginConfigResponse[] = $state([]);
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let submitting: boolean = $state(false);

	onMount(async () => {
		try {
			const result = await getPluginConfigs(1, 500);
			pluginConfigs = result.items;
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load plugin configs.';
		} finally {
			loading = false;
		}
	});

	function validateOverride(role: RoleKey): boolean {
		const text = roleStates[role].config_override_text.trim();
		if (!text) {
			roleStates[role].config_override_error = null;
			return true;
		}
		try {
			const parsed: unknown = JSON.parse(text);
			if (typeof parsed !== 'object' || Array.isArray(parsed) || parsed === null) {
				roleStates[role].config_override_error = 'Must be a JSON object, not an array or primitive.';
				return false;
			}
			roleStates[role].config_override_error = null;
			return true;
		} catch {
			roleStates[role].config_override_error = 'Invalid JSON.';
			return false;
		}
	}

	async function save() {
		if (submitting) return;

		let allValid = true;
		for (const role of ALL_ROLES) {
			if (!validateOverride(role)) allValid = false;
		}
		if (!allValid) return;

		const toUpdate = ALL_ROLES.filter((role) => roleStates[role].plugin_config_id);
		if (toUpdate.length === 0) {
			showError('Select at least one plugin config to save.');
			return;
		}

		submitting = true;
		try {
			let lastResult: SoftwareItemDetailResponse | undefined;
			for (const role of toUpdate) {
				const s = roleStates[role];
				const overrideText = s.config_override_text.trim();
				const req: UpdateHostAssignmentRequest = {
					role,
					plugin_config_id: s.plugin_config_id,
					package_identifier: s.package_identifier.trim() || undefined,
					execution_site: s.execution_site,
					config_override: overrideText ? (JSON.parse(overrideText) as Record<string, unknown>) : null
				};
				lastResult = await updateHostAssignment(softwareItemId, hostId, req);
			}
			showSuccess('Plugin assignments saved.');
			onsuccess(lastResult!);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to save plugin assignments.');
		} finally {
			submitting = false;
		}
	}
</script>

<Modal title="Configure Plugins" {onclose} maxWidth="max-w-2xl max-h-[90vh] flex flex-col">
	<div class="space-y-1 text-sm text-surface-500">
		<p>
			Editing plugin assignments for <strong class="text-surface-900 dark:text-surface-100">{softwareItemName}</strong>
			on <strong class="text-surface-900 dark:text-surface-100">{hostName}</strong>.
		</p>
		<p class="text-xs">To remove a role entirely, unassign this host and reassign without that role.</p>
	</div>

	{#if loading}
		<p class="text-surface-500">Loading plugin configs…</p>
	{:else if loadError}
		<aside class="rounded-lg p-4 preset-filled-error-500 text-sm">{loadError}</aside>
	{:else}
		<div class="space-y-3">
			{#each ALL_ROLES as role (role)}
				{@const s = roleStates[role]}
				<div class="rounded-lg border border-surface-200 p-4 space-y-3 dark:border-surface-700">
					<div class="flex items-start gap-2">
						<span class="badge preset-tonal shrink-0 text-xs">{ROLE_LABELS[role]}</span>
						<span class="text-xs text-surface-500 leading-5">{ROLE_DESCRIPTIONS[role]}</span>
					</div>

					<!-- Plugin Config -->
					<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
						<label class="text-sm font-medium" for="cfg-{role}">Plugin Config</label>
						<select id="cfg-{role}" class="select text-sm" bind:value={roleStates[role].plugin_config_id}>
							<option value="">— not configured —</option>
							{#each pluginConfigs as cfg (cfg.id)}
								<option value={cfg.id}>{cfg.name}</option>
							{/each}
						</select>
					</div>

					{#if s.plugin_config_id}
						<!-- Package Identifier -->
						<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
							<label class="text-sm font-medium" for="pkg-{role}">Package ID</label>
							<input
								id="pkg-{role}"
								class="input text-sm"
								type="text"
								placeholder="e.g. owner/repo"
								bind:value={roleStates[role].package_identifier}
							/>
						</div>

						<!-- Execution Site (only for fetch_releases) -->
						{#if role === 'fetch_releases'}
							<div class="grid grid-cols-[9rem_1fr] items-center gap-3">
								<label class="text-sm font-medium" for="site-{role}">Execution Site</label>
								<select id="site-{role}" class="select text-sm" bind:value={roleStates[role].execution_site}>
									<option value="auto">Auto (recommended)</option>
									<option value="agent">Agent</option>
									<option value="controller">Controller</option>
								</select>
							</div>
						{/if}

						<!-- Config Override (advanced, collapsible) -->
						<details>
							<summary class="cursor-pointer select-none text-xs text-surface-500 hover:text-surface-700">
								Config Override <span class="opacity-60">(advanced)</span>
								{#if s.config_override_text.trim()}
									<span class="ml-1 badge preset-tonal-warning text-xs">set</span>
								{/if}
							</summary>
							<div class="mt-2 space-y-1">
								<textarea
									class="textarea font-mono text-xs"
									rows={4}
									placeholder={`{\n  "example_field": "value"\n}`}
									bind:value={roleStates[role].config_override_text}
									onblur={() => validateOverride(role)}
								></textarea>
								{#if s.config_override_error}
									<p class="text-xs rounded px-2 py-1 preset-filled-error-500">{s.config_override_error}</p>
								{/if}
								<p class="text-xs text-surface-400">
									JSON object merged on top of the plugin config. Leave empty to clear any existing override.
								</p>
							</div>
						</details>
					{/if}
				</div>
			{/each}
		</div>
	{/if}

	{#snippet footer()}
		<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
		<button class="btn preset-filled-primary-500" onclick={save} disabled={submitting || loading || !!loadError}>
			{submitting ? 'Saving…' : 'Save Changes'}
		</button>
	{/snippet}
</Modal>
