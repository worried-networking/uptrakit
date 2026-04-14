<script lang="ts">
	import SurfaceRenderer from './SurfaceRenderer.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import { getSurfaceProviders } from '$lib/surfaces/registry.svelte';
	import { buildStaticSurfaceData, isSurfaceReadRenderable } from '$lib/surfaces/read-model';
	import type { SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import type { DataSourceDescriptor, SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

	let {
		surface,
		read,
		baseParams = {}
	}: {
		surface: SurfaceResponse;
		read?: SurfaceReadResponse;
		baseParams?: Record<string, unknown>;
	} = $props();

	let selectedProviderId = $state<string | undefined>(undefined);
	let hydratedDataBySource = $state<Record<string, unknown>>({});
	let hydrationLoading = $state(false);
	let hydrationError = $state<string | null>(null);
	let completedHydrationFingerprint = $state<string | null>(null);
	const descriptorMismatch = $derived(read ? read.descriptor.surface_id !== surface.surface_id : false);
	const descriptor = $derived(read ? read.descriptor : surface);

	const providers = $derived(getSurfaceProviders(descriptor.surface_id));
	const availableProviders = $derived(providers.filter((provider) => provider.availability === 'available'));
	const selectedProvider = $derived.by(() => {
		if (descriptor.targeting !== 'targeted') {
			return undefined;
		}
		return (
			availableProviders.find((provider) => provider.provider_id === selectedProviderId) ??
			availableProviders[0] ??
			providers[0]
		);
	});
	const encryptionContext = $derived.by<SurfaceEncryptionContext | undefined>(() => {
		const metadata = selectedProvider?.encryption_metadata;
		if (!metadata) {
			return undefined;
		}
		return {
			keyId: metadata.key_id,
			algorithm: metadata.algorithm,
			publicKey: metadata.public_key
		};
	});
	const staticData = $derived(read ? buildStaticSurfaceData(read.data_sources) : {});
	const dataBySource = $derived({
		...staticData,
		...hydratedDataBySource
	});
	const hydrationRequests = $derived(read ? collectProviderQueryHydrationRequests(read) : []);
	const baseParamsFingerprint = $derived(stableStringify(baseParams));
	const hydrationFingerprint = $derived.by(() => {
		if (!read || descriptorMismatch || hydrationRequests.length === 0) {
			return null;
		}
		const targetProviderId = descriptor.targeting === 'targeted' ? (selectedProvider?.provider_id ?? null) : null;
		if (descriptor.targeting === 'targeted' && !targetProviderId) {
			return null;
		}
		return stableStringify({
			surface_id: descriptor.surface_id,
			target_provider_id: targetProviderId,
			requests: hydrationRequests.map((request) => `${request.dataSourceId}:${request.interactionId}`).sort(),
			base_params: baseParamsFingerprint
		});
	});

	$effect(() => {
		if (descriptor.targeting !== 'targeted') {
			selectedProviderId = undefined;
			return;
		}
		if (selectedProviderId && availableProviders.some((provider) => provider.provider_id === selectedProviderId)) {
			return;
		}
		selectedProviderId = availableProviders[0]?.provider_id;
	});

	$effect(() => {
		// Depend on baseParams identity so parent rerenders can trigger retries
		// after failures, while successful fingerprints still dedupe requests.
		const _baseParamsDependency = baseParams;
		void _baseParamsDependency;
		if (!read || descriptorMismatch || hydrationRequests.length === 0) {
			hydratedDataBySource = {};
			hydrationLoading = false;
			hydrationError = null;
			completedHydrationFingerprint = null;
			return;
		}

		const targetProviderId = descriptor.targeting === 'targeted' ? selectedProvider?.provider_id : undefined;
		if (descriptor.targeting === 'targeted' && !targetProviderId) {
			hydratedDataBySource = {};
			hydrationLoading = false;
			hydrationError = null;
			completedHydrationFingerprint = null;
			return;
		}
		if (!hydrationFingerprint || hydrationFingerprint === completedHydrationFingerprint) {
			return;
		}

		let cancelled = false;
		hydrationLoading = true;
		hydrationError = null;
		void (async () => {
			const loadedData: Record<string, unknown> = {};
			let failed = false;
			for (const request of hydrationRequests) {
				try {
					const response = await invokeSurfaceInteraction(descriptor.surface_id, request.interactionId, {
						params: parseRecordFromStableJson(baseParamsFingerprint),
						target_provider_id: targetProviderId
					});
					loadedData[request.dataSourceId] = normalizeKeyValuePayload(response);
				} catch (error) {
					console.error(`Failed to hydrate data source ${request.dataSourceId}:`, error);
					failed = true;
					break;
				}
			}
			if (cancelled) {
				return;
			}
			hydratedDataBySource = failed ? {} : loadedData;
			hydrationError = failed ? 'Failed to load surface data. Please try again.' : null;
			hydrationLoading = false;
			if (!failed) {
				completedHydrationFingerprint = hydrationFingerprint;
			}
		})();

		return () => {
			cancelled = true;
		};
	});

	interface ProviderQueryHydrationRequest {
		dataSourceId: string;
		interactionId: string;
	}

	function collectProviderQueryHydrationRequests(model: SurfaceReadResponse): ProviderQueryHydrationRequest[] {
		const keyValueDataSourceIds = new Set<string>();
		collectKeyValueDataSourceIds(model.descriptor.root_node, keyValueDataSourceIds);

		const requests: ProviderQueryHydrationRequest[] = [];
		for (const dataSourceId of keyValueDataSourceIds) {
			const dataSource = model.data_sources.find((source) => source.data_source_id === dataSourceId);
			if (!isProviderQueryDataSource(dataSource)) {
				continue;
			}
			const interactionId = dataSource.kind.operation_id;
			const interaction = model.interactions.find(
				(candidate) => candidate.interaction_id === interactionId && candidate.kind === 'data_load'
			);
			if (!interaction) {
				continue;
			}
			requests.push({
				dataSourceId,
				interactionId
			});
		}
		return requests;
	}

	function collectKeyValueDataSourceIds(node: SurfaceReadResponse['descriptor']['root_node'], out: Set<string>): void {
		if (node.kind === 'key_value') {
			out.add(node.data_source_id);
			return;
		}
		if (node.kind === 'section') {
			for (const child of node.children ?? []) {
				collectKeyValueDataSourceIds(child, out);
			}
			return;
		}
		if (node.kind === 'tabs') {
			for (const tab of node.tabs ?? []) {
				collectKeyValueDataSourceIds(tab.root, out);
			}
			return;
		}
		if (node.kind === 'modal_trigger') {
			for (const modalNode of node.modal_nodes ?? []) {
				collectKeyValueDataSourceIds(modalNode, out);
			}
			return;
		}
		if (node.kind === 'workflow_trigger') {
			for (const stepNode of node.step_nodes ?? []) {
				collectKeyValueDataSourceIds(stepNode, out);
			}
		}
	}

	function isProviderQueryDataSource(
		dataSource: DataSourceDescriptor | undefined
	): dataSource is DataSourceDescriptor & {
		kind: {
			kind: 'provider_query';
			operation_id: string;
		};
	} {
		return !!dataSource && dataSource.kind.kind === 'provider_query';
	}

	function normalizeKeyValuePayload(value: unknown): Record<string, unknown> {
		if (value && typeof value === 'object' && !Array.isArray(value)) {
			return value as Record<string, unknown>;
		}
		return {};
	}

	function parseRecordFromStableJson(value: string): Record<string, unknown> {
		try {
			const parsed = JSON.parse(value);
			if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
				return parsed as Record<string, unknown>;
			}
		} catch {
			// Fall through to empty object.
		}
		return {};
	}

	function stableStringify(value: unknown): string {
		if (value === null) {
			return 'null';
		}
		if (typeof value !== 'object') {
			return JSON.stringify(value);
		}
		if (Array.isArray(value)) {
			return `[${value.map((item) => stableStringify(item)).join(',')}]`;
		}
		const entries = Object.entries(value as Record<string, unknown>).sort(([left], [right]) =>
			left.localeCompare(right)
		);
		return `{${entries.map(([key, entryValue]) => `${JSON.stringify(key)}:${stableStringify(entryValue)}`).join(',')}}`;
	}
</script>

{#if !read}
	<aside class="rounded-lg p-4 preset-filled-warning-500">Surface contract is not available yet.</aside>
{:else if descriptorMismatch}
	<aside class="rounded-lg p-4 preset-filled-warning-500">
		Surface contract mismatch detected. Please refresh and try again.
	</aside>
{:else if !isSurfaceReadRenderable(read)}
	<aside class="rounded-lg p-4 preset-filled-warning-500">
		This surface uses unsupported data sources and cannot be rendered yet.
	</aside>
{:else if hydrationLoading}
	<p class="py-8 text-center text-surface-500">Loading...</p>
{:else if hydrationError}
	<aside class="rounded-lg p-4 preset-filled-error-500">{hydrationError}</aside>
{:else if descriptor.targeting === 'targeted'}
	{#if availableProviders.length === 0}
		<p class="py-8 text-center text-surface-500">No compatible provider is currently connected for this surface.</p>
	{:else}
		<div class="mb-4">
			<label class="label">
				<span>Provider</span>
				<select class="select" bind:value={selectedProviderId}>
					{#each availableProviders as provider (provider.provider_id)}
						<option value={provider.provider_id}>{provider.display_label}</option>
					{/each}
				</select>
			</label>
		</div>
		<SurfaceRenderer
			surfaceId={descriptor.surface_id}
			node={descriptor.root_node}
			interactions={read.interactions}
			targetProviderId={selectedProvider?.provider_id}
			{encryptionContext}
			{dataBySource}
			{baseParams}
		/>
	{/if}
{:else}
	<SurfaceRenderer
		surfaceId={descriptor.surface_id}
		node={descriptor.root_node}
		interactions={read.interactions}
		{dataBySource}
		{baseParams}
	/>
{/if}
