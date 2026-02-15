<script lang="ts">
	import { onMount } from 'svelte';
	import { user } from '$lib/auth';
	import { getSoftwareItems, getProviderConfigs, createSoftwareItem } from '$lib/api';
	import { showError, showSuccess, clearError } from '$lib/notifications.svelte';
	import { formatDate } from '$lib/utils';
	import Pagination from '$lib/components/Pagination.svelte';
	import type { ProviderConfigResponse, SoftwareItemResponse } from '$lib/types';
	import { Permission } from '$lib/types';

	let items: SoftwareItemResponse[] = $state([]);
	let providerConfigs: ProviderConfigResponse[] = $state([]);
	let error: string | null = $state(null);
	let configError: string | null = $state(null);
	let providerConfigError: string | null = $state(null);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let loading: boolean = $state(false);
	let submitting: boolean = $state(false);

	let name: string = $state('');
	let providerConfigId: string = $state('');
	let createNewConfig: boolean = $state(false);
	let providerConfigName: string = $state('');
	let providerConfigType: string = $state('github_releases');
	let providerConfigText: string = $state('');
	let providerConfigEnabled: boolean = $state(true);
	let packageIdentifier: string = $state('');
	let enabled: boolean = $state(true);
	let configOverrideText: string = $state('');

	const canView = $derived($user?.permissions.includes(Permission.ViewSettings) ?? false);
	const canManage = $derived($user?.permissions.includes(Permission.ManageSettings) ?? false);

	const homebrewConfigs = $derived(
		providerConfigs.filter((config) => config.provider_type === 'homebrew')
	);
	const canToggleCreate = $derived(providerConfigs.length > 0);

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

	onMount(() => {
		if (canView) {
			loadAll(1);
		}
	});

	async function loadAll(page: number) {
		loading = true;
		try {
			error = null;
			const [itemsPromise, configsPromise] = await Promise.allSettled([
				getSoftwareItems(page),
				getProviderConfigs(1, 500)
			]);

			if (itemsPromise.status === 'fulfilled') {
				items = itemsPromise.value.items;
				currentPage = itemsPromise.value.page;
				totalPages = itemsPromise.value.total_pages;
				error = null; // Clear previous error if items loaded successfully
			} else {
				error = itemsPromise.reason instanceof Error ? itemsPromise.reason.message : 'Failed to load software items.';
			}

			if (configsPromise.status === 'fulfilled') {
				providerConfigs = configsPromise.value.items;
				providerConfigError = null; // Clear previous error if provider configs loaded successfully
				if (!providerConfigId) {
					const preferred = configsPromise.value.items.find((config) => config.provider_type === 'homebrew');
					providerConfigId = preferred?.id ?? configsPromise.value.items[0]?.id ?? '';
				}
				if (configsPromise.value.items.length === 0) {
					createNewConfig = true;
					ensureProviderConfigTemplate();
				}
			} else {
				providerConfigError = configsPromise.reason instanceof Error ? configsPromise.reason.message : 'Failed to load provider configurations.';
			}
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load software items';
		} finally {
			loading = false;
		}
	}

	function providerConfigTemplate(type: string): string {
		const template = providerConfigTemplates[type] ?? {};
		return JSON.stringify(template, null, 2);
	}

	function ensureProviderConfigTemplate() {
		if (!providerConfigText.trim()) {
			providerConfigText = providerConfigTemplate(providerConfigType);
		}
	}

	function parseConfigOverride(): Record<string, unknown> | null {
		if (!configOverrideText.trim()) return null;
		try {
			const parsed = JSON.parse(configOverrideText);
			if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
				throw new Error('Config override must be a JSON object.');
			}
			configError = null;
			return parsed as Record<string, unknown>;
		} catch (e) {
			configError = e instanceof Error ? e.message : 'Invalid JSON.';
			return null;
		}
	}

	function parseProviderConfig(): Record<string, unknown> | null {
		if (!providerConfigText.trim()) {
			providerConfigError = null;
			return {};
		}
		try {
			const parsed = JSON.parse(providerConfigText);
			if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
				throw new Error('Provider config must be a JSON object.');
			}
			providerConfigError = null;
			return parsed as Record<string, unknown>;
		} catch (e) {
			providerConfigError = e instanceof Error ? e.message : 'Invalid JSON.';
			return null;
		}
	}

	async function submitSoftware() {
		if (!canManage || submitting) return;
		const trimmedName = name.trim();
		const trimmedPackage = packageIdentifier.trim();
		if (!trimmedName) {
			error = 'Name is required.';
			showError(error);
			return;
		}
		let providerConfigPayload: Record<string, unknown> | null = null;
		let providerConfigNameValue = '';
		if (createNewConfig) {
			providerConfigNameValue = providerConfigName.trim();
			if (!providerConfigNameValue) {
				error = 'Provider config name is required.';
				showError(error);
				return;
			}
			providerConfigPayload = parseProviderConfig();
			if (!providerConfigPayload) {
				showError(providerConfigError ?? 'Invalid provider config JSON.');
				return;
			}
		} else {
			if (!providerConfigId) {
				error = 'Provider config is required.';
				showError(error);
				return;
			}
		}

		const overrideValue = parseConfigOverride();
		if (configOverrideText.trim() && !overrideValue) {
			showError(configError ?? 'Invalid config override JSON.');
			return;
		}

		submitting = true;
		try {
			error = null;
			clearError();
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
			showSuccess('Software item registered.');
			name = '';
			packageIdentifier = '';
			configOverrideText = '';
			enabled = true;
			providerConfigName = '';
			providerConfigText = '';
			providerConfigEnabled = true;
			providerConfigId = created.provider_config_id ?? providerConfigId;
			createNewConfig = false;
			await loadAll(1);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to register software item';
			showError(error);
		} finally {
			submitting = false;
		}
	}
</script>

{#if $user}
	<h1 class="h1 mb-4">Software</h1>

	{#if !canView}
		<aside class="rounded-lg p-4 preset-filled-error-500">
			<p>You do not have permission to view software items.</p>
		</aside>
	{:else}
		{#if canManage}
			<section class="mb-6">
				<div class="card space-y-4 p-6">
					<div>
						<h2 class="h3">Register Software</h2>
						<p class="text-sm text-surface-500">
							Select an existing provider config or create one while registering software. Updates are still manual.
						</p>
					</div>

					{#if providerConfigs.length === 0}
						<aside class="rounded-lg p-4 preset-filled-warning-500">
							<p>No provider configs are available yet. Create one below to continue.</p>
						</aside>
					{/if}

					<div class="grid gap-4 md:grid-cols-2">
						<label class="label">
							<span>Name</span>
							<input class="input" bind:value={name} placeholder="Firefox" />
						</label>
						<label class="label">
							<span>Provider config</span>
							{#if createNewConfig}
								<select class="select" bind:value={providerConfigType} onchange={ensureProviderConfigTemplate}>
									{#each providerTypeOptions as option}
										<option value={option.value}>{option.label}</option>
									{/each}
								</select>
							{:else}
								<select class="select" bind:value={providerConfigId}>
									{#each providerConfigs as config (config.id)}
										<option value={config.id}>
											{config.name} ({config.provider_type})
										</option>
									{/each}
								</select>
							{/if}
						</label>
						<label class="label">
							<span>Package identifier</span>
							<input class="input" bind:value={packageIdentifier} placeholder="firefox" />
						</label>
						<label class="label">
							<span>Enabled</span>
							<div class="flex items-center gap-2">
								<input type="checkbox" class="checkbox" bind:checked={enabled} />
								<span class="text-sm">Track updates for this item</span>
							</div>
						</label>
					</div>

					<label class="label">
						<span>Create new provider config</span>
						<div class="flex items-center gap-2">
							<input
								type="checkbox"
								class="checkbox"
								bind:checked={createNewConfig}
								disabled={!canToggleCreate}
								onchange={ensureProviderConfigTemplate}
							/>
							<span class="text-sm">
								{canToggleCreate ? 'Create a new config for this software' : 'Create a new config (required)'}
							</span>
						</div>
					</label>

					{#if createNewConfig}
						<div class="grid gap-4 md:grid-cols-2">
							<label class="label">
								<span>Provider config name</span>
								<input class="input" bind:value={providerConfigName} placeholder="GitHub Releases" />
							</label>
							<label class="label">
								<span>Provider config enabled</span>
								<div class="flex items-center gap-2">
									<input type="checkbox" class="checkbox" bind:checked={providerConfigEnabled} />
									<span class="text-sm">Allow this config to run checks</span>
								</div>
							</label>
						</div>
						<label class="label">
							<span>Provider config (JSON)</span>
							<textarea
								class="textarea"
								rows="4"
								bind:value={providerConfigText}
								placeholder={providerConfigTemplate(providerConfigType)}
							></textarea>
						</label>
						{#if providerConfigError}
							<p class="text-sm text-error-500">{providerConfigError}</p>
						{/if}
					{/if}

					<label class="label">
						<span>Config override (JSON, optional)</span>
						<textarea
							class="textarea"
							rows="4"
							bind:value={configOverrideText}
							placeholder={'{"asset_patterns": [".*linux"]}'}
						></textarea>
					</label>
					{#if configError}
						<p class="text-sm text-error-500">{configError}</p>
					{/if}
					{#if homebrewConfigs.length > 0}
						<p class="text-xs text-surface-500">
							Homebrew package identifiers should be formula or cask names (e.g. "wget", "homebrew/cask/firefox").
						</p>
					{/if}
					<div class="flex justify-end">
						<button
							class="btn preset-filled-primary-500"
							disabled={submitting}
							onclick={submitSoftware}
						>
							{submitting ? 'Registering...' : 'Register Software'}
						</button>
					</div>
				</div>
			</section>
		{/if}

		{#if error}
			<aside class="mb-4 rounded-lg p-4 preset-filled-error-500">
				<p>{error}</p>
			</aside>
		{/if}

		<div class="table-wrap">
			<table class="table">
				<thead>
					<tr>
						<th>Name</th>
						<th>Provider</th>
						<th>Package</th>
						<th>Status</th>
						<th>Hosts</th>
						<th>Last Checked</th>
					</tr>
				</thead>
				<tbody>
					{#if loading}
						<tr>
							<td colspan="6" class="py-6 text-center">Loading...</td>
						</tr>
					{:else}
						{#each items as item (item.id)}
							<tr>
								<td>{item.name}</td>
								<td>
									<span class="badge preset-tonal">{item.provider_config_name}</span>
								</td>
								<td>{item.package_identifier || '\u2014'}</td>
								<td>
									{#if item.enabled}
										<span class="badge preset-filled-success-500">Enabled</span>
									{:else}
										<span class="badge preset-tonal">Disabled</span>
									{/if}
								</td>
								<td>{item.host_count}</td>
								<td>{formatDate(item.last_checked_at)}</td>
							</tr>
						{:else}
							<tr>
								<td colspan="6" class="py-8 text-center">
									<p class="text-lg font-medium">No software registered yet</p>
									<p class="mt-1 text-sm text-surface-500">
										Register a package to start tracking.
									</p>
								</td>
							</tr>
						{/each}
					{/if}
				</tbody>
			</table>
		</div>

		<Pagination {currentPage} {totalPages} onPageChange={loadAll} />
	{/if}
{/if}
