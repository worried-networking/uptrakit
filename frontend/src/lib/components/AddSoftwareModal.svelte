<script lang="ts">
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import { getProviderConfigs, createSoftwareItem, getHosts, assignHostsToSoftwareItem } from '$lib/api';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { HostResponse, ProviderConfigResponse, SoftwareItemResponse } from '$lib/types';

	let {
		onclose,
		onsuccess
	}: {
		onclose: () => void;
		onsuccess: (created: SoftwareItemResponse) => void;
	} = $props();

	let name: string = $state('');
	let packageIdentifier: string = $state('');
	let enabled: boolean = $state(true);
	let providerConfigId: string = $state('');
	let createNewConfig: boolean = $state(false);
	let providerConfigName: string = $state('');
	let providerConfigType: string = $state('github_releases');
	let providerConfigText: string = $state('');
	let providerConfigEnabled: boolean = $state(true);
	let newConfigError: string | null = $state(null);
	let showConfigOverride: boolean = $state(false);
	let configOverrideText: string = $state('');
	let configOverrideError: string | null = $state(null);
	let submitting: boolean = $state(false);
	let configsLoading: boolean = $state(false);
	let providerConfigs: ProviderConfigResponse[] = $state([]);
	let configsLoadError: string | null = $state(null);

	// Host assignment
	let showAssignHosts: boolean = $state(false);
	let allHosts: HostResponse[] = $state([]);
	let hostsLoading: boolean = $state(false);
	let hostsLoadFailed: boolean = $state(false);
	const selectedHostIds = new SvelteSet<string>();

	const providerTypeOptions = [
		{ value: 'github_releases', label: 'GitHub Releases' },
		{ value: 'docker_registry', label: 'Docker Registry' },
		{ value: 'proxmox_helper_scripts', label: 'Proxmox Helper Scripts' },
		{ value: 'homebrew', label: 'Homebrew' }
	];

	const providerConfigTemplates: Record<string, Record<string, unknown>> = {
		github_releases: { owner: 'octocat', repo: 'hello-world' },
		docker_registry: { image: 'nginx' },
		proxmox_helper_scripts: {},
		homebrew: { package_type: 'formula' }
	};

	const packageIdentifierHelp: Record<string, string> = {
		github_releases: 'Asset filter pattern — leave blank to match any release asset.',
		docker_registry: 'Image tag or digest — leave blank to track `latest`.',
		proxmox_helper_scripts: 'Script name in the Proxmox Helper Scripts index. Leave blank to auto-detect.',
		homebrew: 'Formula or cask name, e.g. `wget` or `homebrew/cask/firefox`.'
	};

	const activeProviderType = $derived(
		createNewConfig ? providerConfigType : (providerConfigs.find((c) => c.id === providerConfigId)?.provider_type ?? '')
	);

	onMount(async () => {
		configsLoading = true;
		hostsLoading = true;
		try {
			const [configsResult, hostsResult] = await Promise.allSettled([getProviderConfigs(1, 500), getHosts(1, 200)]);

			if (configsResult.status === 'fulfilled') {
				providerConfigs = configsResult.value.items;
				providerConfigId = configsResult.value.items[0]?.id ?? '';
				if (configsResult.value.items.length === 0) {
					createNewConfig = true;
					providerConfigText = providerConfigTemplate(providerConfigType);
				}
			} else {
				configsLoadError =
					configsResult.reason instanceof Error
						? configsResult.reason.message
						: 'Failed to load provider configurations.';
			}

			if (hostsResult.status === 'fulfilled') {
				allHosts = hostsResult.value.items;
			} else {
				hostsLoadFailed = true;
			}
		} finally {
			configsLoading = false;
			hostsLoading = false;
		}
	});

	function providerConfigTemplate(type: string): string {
		const template = providerConfigTemplates[type] ?? {};
		return JSON.stringify(template, null, 2);
	}

	function onProviderTypeChange() {
		providerConfigText = providerConfigTemplate(providerConfigType);
		newConfigError = null;
	}

	function parseConfigOverride(): Record<string, unknown> | null {
		if (!configOverrideText.trim()) return null;
		try {
			const parsed = JSON.parse(configOverrideText);
			if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
				throw new Error('Config override must be a JSON object.');
			}
			configOverrideError = null;
			return parsed as Record<string, unknown>;
		} catch (e) {
			configOverrideError = e instanceof Error ? e.message : 'Invalid JSON.';
			return null;
		}
	}

	function parseProviderConfig(): Record<string, unknown> | null {
		if (!providerConfigText.trim()) {
			newConfigError = null;
			return {};
		}
		try {
			const parsed = JSON.parse(providerConfigText);
			if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
				throw new Error('Provider config must be a JSON object.');
			}
			newConfigError = null;
			return parsed as Record<string, unknown>;
		} catch (e) {
			newConfigError = e instanceof Error ? e.message : 'Invalid JSON.';
			return null;
		}
	}

	function toggleHost(hostId: string) {
		if (selectedHostIds.has(hostId)) {
			selectedHostIds.delete(hostId);
		} else {
			selectedHostIds.add(hostId);
		}
	}

	async function submit() {
		if (submitting) return;
		const trimmedName = name.trim();
		const trimmedPackage = packageIdentifier.trim();
		if (!trimmedName) {
			showError('Name is required.');
			return;
		}
		let providerConfigPayload: Record<string, unknown> | null = null;
		let providerConfigNameValue = '';
		if (createNewConfig) {
			providerConfigNameValue = providerConfigName.trim();
			if (!providerConfigNameValue) {
				showError('Provider config name is required.');
				return;
			}
			providerConfigPayload = parseProviderConfig();
			if (!providerConfigPayload) {
				showError(newConfigError ?? 'Invalid provider config JSON.');
				return;
			}
		} else {
			if (!providerConfigId) {
				showError('Provider config is required.');
				return;
			}
		}

		const overrideValue = parseConfigOverride();
		if (configOverrideText.trim() && !overrideValue) {
			showError(configOverrideError ?? 'Invalid config override JSON.');
			return;
		}

		submitting = true;
		try {
			const created = await createSoftwareItem({
				name: trimmedName,
				provider_config_id: createNewConfig ? undefined : providerConfigId,
				provider_config: createNewConfig
					? {
							name: providerConfigNameValue,
							provider_type: providerConfigType,
							config: providerConfigPayload ?? {},
							enabled: providerConfigEnabled
						}
					: undefined,
				package_identifier: trimmedPackage || undefined,
				config_override: overrideValue ?? undefined,
				enabled
			});

			const hostIds = [...selectedHostIds];
			if (hostIds.length > 0) {
				try {
					await assignHostsToSoftwareItem(created.id, { host_ids: hostIds });
				} catch {
					showError(
						'Software registered, but host assignment failed. You can assign hosts later from the context menu.'
					);
					onsuccess(created);
					return;
				}
			}

			showSuccess('Software item registered.');
			onsuccess(created);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to register software item');
		} finally {
			submitting = false;
		}
	}
</script>

<ModalBackdrop {onclose}>
	<div
		class="card bg-surface-50 dark:bg-surface-900 w-full max-w-2xl max-h-[90vh] space-y-4 overflow-y-auto p-6 shadow-xl"
		role="dialog"
		aria-modal="true"
		aria-labelledby="add-software-title"
	>
		<div>
			<h3 class="h3" id="add-software-title">Add Software</h3>
			<p class="text-sm text-surface-500">Register a software item to start tracking updates.</p>
		</div>

		{#if configsLoading}
			<p class="text-surface-500">Loading provider configurations...</p>
		{:else if configsLoadError}
			<aside class="rounded-lg p-4 preset-filled-error-500">
				<p>{configsLoadError}</p>
			</aside>
		{:else}
			<div class="grid gap-4 md:grid-cols-2">
				<label class="label">
					<span>Name</span>
					<input class="input" bind:value={name} placeholder="Firefox" />
				</label>
				<div class="label">
					<label for="pkg-id-input">Package identifier</label>
					<input id="pkg-id-input" class="input" bind:value={packageIdentifier} placeholder="firefox" />
					{#if activeProviderType && packageIdentifierHelp[activeProviderType]}
						<p class="mt-1 text-xs text-surface-500">{packageIdentifierHelp[activeProviderType]}</p>
					{/if}
				</div>
			</div>

			<label class="flex items-center gap-3">
				<input class="checkbox" type="checkbox" bind:checked={enabled} />
				<span>Track updates for this item</span>
			</label>

			<div class="space-y-3">
				<h4 class="font-medium">Provider Configuration</h4>

				{#if providerConfigs.length === 0}
					<aside class="rounded-lg p-3 preset-filled-warning-500">
						<p class="text-sm">No provider configs exist yet. Create one below to continue.</p>
					</aside>
				{/if}

				{#if !createNewConfig && providerConfigs.length > 0}
					<label class="label">
						<span>Select provider config</span>
						<select class="select" bind:value={providerConfigId}>
							{#each providerConfigs as config (config.id)}
								<option value={config.id}>{config.name} ({config.provider_type})</option>
							{/each}
						</select>
					</label>
					<button
						type="button"
						class="text-sm text-primary-500 underline hover:no-underline"
						onclick={() => {
							createNewConfig = true;
							if (!providerConfigText.trim()) {
								providerConfigText = providerConfigTemplate(providerConfigType);
							}
						}}
					>
						Create a new provider config instead
					</button>
				{:else if createNewConfig}
					{#if providerConfigs.length > 0}
						<button
							type="button"
							class="text-sm text-primary-500 underline hover:no-underline"
							onclick={() => {
								createNewConfig = false;
								newConfigError = null;
							}}
						>
							Use an existing provider config instead
						</button>
					{/if}

					<div class="grid gap-4 md:grid-cols-2">
						<label class="label">
							<span>Config name</span>
							<input class="input" bind:value={providerConfigName} placeholder="GitHub Releases" />
						</label>
						<label class="label">
							<span>Provider type</span>
							<select class="select" bind:value={providerConfigType} onchange={onProviderTypeChange}>
								{#each providerTypeOptions as option (option.value)}
									<option value={option.value}>{option.label}</option>
								{/each}
							</select>
						</label>
					</div>

					<label class="flex items-center gap-3">
						<input class="checkbox" type="checkbox" bind:checked={providerConfigEnabled} />
						<span>Allow this config to run checks</span>
					</label>

					<label class="label">
						<span>Provider config (JSON)</span>
						<textarea
							class="textarea font-mono text-sm"
							rows="4"
							bind:value={providerConfigText}
							placeholder={providerConfigTemplate(providerConfigType)}
						></textarea>
					</label>
					{#if newConfigError}
						<p class="text-sm text-error-500">{newConfigError}</p>
					{/if}
				{/if}
			</div>

			<div class="space-y-2">
				<button
					type="button"
					class="text-sm text-surface-500 hover:text-surface-700 dark:hover:text-surface-300"
					onclick={() => {
						showConfigOverride = !showConfigOverride;
					}}
				>
					{showConfigOverride ? 'Hide config override ▲' : 'Show config override ▼'}
				</button>
				{#if showConfigOverride}
					<label class="label">
						<span>Config override (JSON, optional)</span>
						<textarea
							class="textarea font-mono text-sm"
							rows="4"
							bind:value={configOverrideText}
							placeholder={'{"asset_patterns": [".*linux"]}'}
						></textarea>
					</label>
					{#if configOverrideError}
						<p class="text-sm text-error-500">{configOverrideError}</p>
					{/if}
				{/if}
			</div>

			<div class="space-y-2">
				<button
					type="button"
					class="text-sm text-surface-500 hover:text-surface-700 dark:hover:text-surface-300"
					onclick={() => (showAssignHosts = !showAssignHosts)}
				>
					{showAssignHosts ? 'Hide host assignment ▲' : 'Assign to hosts (optional) ▼'}
				</button>
				{#if showAssignHosts}
					{#if hostsLoadFailed}
						<aside class="rounded-lg p-3 preset-tonal-surface">
							<p class="text-sm text-surface-500">Host list unavailable. You can assign hosts after registration.</p>
						</aside>
					{:else if hostsLoading}
						<p class="text-sm text-surface-500">Loading hosts...</p>
					{:else if allHosts.length === 0}
						<aside class="rounded-lg p-3 preset-tonal-surface">
							<p class="text-sm text-surface-500">No hosts registered yet. You can assign hosts after registration.</p>
						</aside>
					{:else}
						<ul
							class="max-h-48 overflow-y-auto space-y-0.5 rounded-lg border border-surface-200 dark:border-surface-700 p-2"
						>
							{#each allHosts as host (host.id)}
								<li>
									<label
										class="flex items-center gap-3 rounded-md px-2 py-1.5 cursor-pointer hover:bg-surface-100 dark:hover:bg-surface-800"
									>
										<input
											class="checkbox"
											type="checkbox"
											checked={selectedHostIds.has(host.id)}
											onchange={() => toggleHost(host.id)}
										/>
										<span class="flex-1 min-w-0">
											<span class="block text-sm font-medium truncate">{host.friendly_name}</span>
											<span class="block text-xs text-surface-500 truncate">{host.hostname}</span>
										</span>
									</label>
								</li>
							{/each}
						</ul>
					{/if}
				{/if}
			</div>
		{/if}

		<div class="flex justify-end gap-2">
			<button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
			<button class="btn preset-filled-primary-500" disabled={submitting || configsLoading} onclick={submit}>
				{submitting ? 'Registering...' : 'Register Software'}
			</button>
		</div>
	</div>
</ModalBackdrop>
