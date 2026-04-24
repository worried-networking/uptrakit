<script lang="ts">
	import SurfaceRenderer from './SurfaceRenderer.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import Button from '$lib/components/Button.svelte';
	import Callout from '$lib/components/ui/Callout.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import ProviderSelector from '$lib/components/ui/ProviderSelector.svelte';
	import { getSurfaceProviders } from '$lib/surfaces/registry.svelte';
	import { buildStaticSurfaceData, isSurfaceReadRenderable } from '$lib/surfaces/read-model';
	import type { SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import type { DataSourceDescriptor, SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

	let {
		surface,
		read,
		baseParams = {},
		reloadToken = 0
	}: {
		surface: SurfaceResponse;
		read?: SurfaceReadResponse;
		baseParams?: Record<string, unknown>;
		reloadToken?: string | number;
	} = $props();

	let selectedProviderId = $state<string | undefined>(undefined);
	let hydratedDataBySource = $state<Record<string, unknown>>({});
	let hydrationLoading = $state(false);
	let hydrationError = $state<string | null>(null);
	let hydrationRetryNonce = $state(0);
	let selectedContextValue = $state('');
	let selectorOptions = $state<{ id: string; label: string }[]>([]);
	const descriptorMismatch = $derived(read ? read.descriptor.surface_id !== surface.surface_id : false);
	const descriptor = $derived(read ? read.descriptor : surface);
	const hydratedCacheByFingerprint: Record<string, Record<string, unknown>> = {};
	const settledReloadKeyByFingerprint: Record<string, string> = {};
	const inFlightReloadKeyByFingerprint: Record<string, string> = {};

	const providers = $derived(getSurfaceProviders(descriptor.surface_id));
	const availableProviders = $derived(providers.filter((provider) => provider.availability === 'available'));
	const selectedProvider = $derived.by(() => {
		if (descriptor.targeting !== 'targeted') {
			return undefined;
		}
		return availableProviders.find((provider) => provider.provider_id === selectedProviderId) ?? availableProviders[0];
	});
	const contextSelector = $derived(descriptor.context_selector);

	const effectiveBaseParams = $derived(
		contextSelector && selectedContextValue
			? { ...baseParams, [contextSelector.param_key]: selectedContextValue }
			: { ...baseParams }
	);

	const requiredContextParam = $derived(contextSelector?.param_key);
	const requiredForInteractionIds = $derived(contextSelector?.required_for_interactions ?? []);

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
	const baseParamsFingerprint = $derived(stableStringify(effectiveBaseParams));
	const reloadTokenFingerprint = $derived(stableStringify({ reloadToken, hydrationRetryNonce }));
	const contractMismatchMessage = 'Surface contract mismatch detected. Please refresh and try again.';
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
		if (!read || descriptorMismatch || hydrationRequests.length === 0) {
			hydratedDataBySource = {};
			hydrationLoading = false;
			hydrationError = null;
			return;
		}

		const targetProviderId = descriptor.targeting === 'targeted' ? selectedProvider?.provider_id : undefined;
		if (descriptor.targeting === 'targeted' && !targetProviderId) {
			hydratedDataBySource = {};
			hydrationLoading = false;
			hydrationError = null;
			return;
		}
		if (!hydrationFingerprint) {
			return;
		}

		const currentHydrationFingerprint = hydrationFingerprint;
		const cached = hydratedCacheByFingerprint[currentHydrationFingerprint];
		if (cached) {
			hydratedDataBySource = cached;
			hydrationError = null;
		}

		const reloadKey = `${currentHydrationFingerprint}|${reloadTokenFingerprint}`;
		if (settledReloadKeyByFingerprint[currentHydrationFingerprint] === reloadKey) {
			hydrationError = null;
			if (!cached) {
				hydratedDataBySource = {};
			}
			hydrationLoading = false;
			return;
		}

		if (inFlightReloadKeyByFingerprint[currentHydrationFingerprint] === reloadKey) {
			hydrationLoading = true;
			return;
		}

		inFlightReloadKeyByFingerprint[currentHydrationFingerprint] = reloadKey;
		const requestParams = parseRecordFromStableJson(baseParamsFingerprint);
		hydrationLoading = true;
		hydrationError = null;
		void (async () => {
			const loadedData: Record<string, unknown> = {};
			let failureMessage: string | null = null;
			for (const request of hydrationRequests) {
				try {
					const response = await invokeSurfaceInteraction(descriptor.surface_id, request.interactionId, {
						params: requestParams,
						target_provider_id: targetProviderId
					});
					loadedData[request.dataSourceId] = normalizeKeyValuePayload(response);
				} catch (error) {
					console.error(`Failed to hydrate data source ${request.dataSourceId}:`, error);
					failureMessage = 'Failed to load surface data. Please try again.';
					break;
				}
			}
			if (inFlightReloadKeyByFingerprint[currentHydrationFingerprint] === reloadKey) {
				delete inFlightReloadKeyByFingerprint[currentHydrationFingerprint];
			}

			if (!failureMessage) {
				settledReloadKeyByFingerprint[currentHydrationFingerprint] = reloadKey;
				hydratedCacheByFingerprint[currentHydrationFingerprint] = loadedData;
			}

			const currentRunKey = hydrationFingerprint ? `${hydrationFingerprint}|${reloadTokenFingerprint}` : null;
			if (currentRunKey !== reloadKey) {
				return;
			}

			if (!failureMessage) {
				hydratedDataBySource = loadedData;
			} else if (!cached) {
				hydratedDataBySource = {};
			}
			hydrationError = failureMessage;
			hydrationLoading = false;
		})();
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
		if (value === undefined) {
			return 'null';
		}
		if (typeof value !== 'object') {
			return JSON.stringify(value);
		}
		if (Array.isArray(value)) {
			return `[${value.map((item) => stableStringify(item === undefined ? null : item)).join(',')}]`;
		}
		const entries = Object.entries(value as Record<string, unknown>)
			.filter(([, entryValue]) => entryValue !== undefined)
			.sort(([left], [right]) => left.localeCompare(right));
		return `{${entries.map(([key, entryValue]) => `${JSON.stringify(key)}:${stableStringify(entryValue)}`).join(',')}}`;
	}

	$effect(() => {
		const cs = contextSelector;
		if (!cs) {
			selectorOptions = [];
			selectedContextValue = '';
			return;
		}
		let cancelled = false;
		void (async () => {
			try {
				const response = await fetch(cs.rest_api_path, { credentials: 'same-origin' });
				if (cancelled) return;
				if (!response.ok) {
					selectorOptions = [];
					return;
				}
				const data: unknown = await response.json();
				if (cancelled) return;
				let rawItems: unknown[] = [];
				if (Array.isArray(data)) {
					rawItems = data;
				} else if (
					data &&
					typeof data === 'object' &&
					'items' in data &&
					Array.isArray((data as { items: unknown[] }).items)
				) {
					rawItems = (data as { items: unknown[] }).items;
				}
				selectorOptions = rawItems
					.filter((item): item is Record<string, unknown> => !!item && typeof item === 'object' && !Array.isArray(item))
					.map((item) => ({
						id: String(item[cs.value_field] ?? ''),
						label: String(item[cs.label_field] ?? '')
					}))
					.filter((opt) => opt.id);
			} catch {
				selectorOptions = [];
			}
		})();
		return () => {
			cancelled = true;
		};
	});

	function retryHydration(): void {
		hydrationRetryNonce += 1;
		hydrationError = null;
	}
</script>

{#if !read}
	<Callout tone="warning" message={contractMismatchMessage} />
{:else if descriptorMismatch}
	<Callout tone="warning" message={contractMismatchMessage} />
{:else if !isSurfaceReadRenderable(read)}
	<Callout tone="danger" message="Surface content could not be rendered." />
{:else if descriptor.targeting === 'targeted'}
	{#if availableProviders.length === 0}
		<EmptyState title="No provider connected" description="Connect a compatible service to use this surface." />
	{:else}
		<div class="mb-4 max-w-[280px]">
			<ProviderSelector
				label="Provider"
				selectedId={selectedProviderId}
				providers={availableProviders.map((provider) => ({
					id: provider.provider_id,
					label: provider.display_label,
					description: undefined
				}))}
				onSelect={(providerId) => {
					selectedProviderId = providerId;
				}}
			/>
		</div>
		{#if hydrationLoading}
			<p class="py-8 text-center text-[var(--text-muted)]">Loading...</p>
		{:else if hydrationError}
			<Callout tone="danger" title="Unable to load surface data" message={hydrationError}>
				<Button variant="danger" size="sm" type="button" onclick={retryHydration}>Try again</Button>
			</Callout>
		{:else}
			<SurfaceRenderer
				surfaceId={descriptor.surface_id}
				node={descriptor.root_node}
				interactions={read.interactions}
				dataSources={read.data_sources}
				targetProviderId={selectedProvider?.provider_id}
				{encryptionContext}
				{dataBySource}
				baseParams={effectiveBaseParams}
			/>
		{/if}
	{/if}
{:else}
	{#if contextSelector}
		<div class="mb-4 max-w-[280px]">
			<ProviderSelector
				label={contextSelector.label}
				providers={[{ id: '', label: contextSelector.all_option_label }, ...selectorOptions]}
				selectedId={selectedContextValue}
				onSelect={(id) => {
					selectedContextValue = id;
				}}
			/>
		</div>
	{/if}
	{#if hydrationLoading}
		<p class="py-8 text-center text-[var(--text-muted)]">Loading...</p>
	{:else if hydrationError}
		<Callout tone="danger" title="Unable to load surface data" message={hydrationError}>
			<Button variant="danger" size="sm" type="button" onclick={retryHydration}>Try again</Button>
		</Callout>
	{:else}
		<SurfaceRenderer
			surfaceId={descriptor.surface_id}
			node={descriptor.root_node}
			interactions={read.interactions}
			dataSources={read.data_sources}
			{dataBySource}
			baseParams={effectiveBaseParams}
			{requiredContextParam}
			{requiredForInteractionIds}
		/>
	{/if}
{/if}
