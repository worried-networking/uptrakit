<script lang="ts">
	import { page } from '$app/state';
	import { getUser } from '$lib/auth.svelte';
	import ExtensionTabContent from '$lib/components/extensions/ExtensionTabContent.svelte';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import { getExtensions, getExtensionsLoaded } from '$lib/extensions.svelte';
	import {
		getSurfaceById,
		getSurfaceReadLoading,
		getSurfaceReadModel,
		getSurfaceReadRequested,
		getSurfaceRegistryLoaded,
		getSurfaceRuntimeStatus,
		loadSurfaceReadModels
	} from '$lib/surfaces/registry.svelte';
	import { isSurfaceTabPending, shouldUseSurfaceRoute } from '$lib/surfaces/read-model';
	import { hasPermissionValue } from '$lib/types';

	let surfaceId = $derived(page.params.id as string);
	let legacyExtension = $derived(getExtensions().find((extension) => extension.id === surfaceId));
	let canViewLegacy = $derived(hasPermissionValue(getUser(), legacyExtension?.required_permission));
	let surface = $derived(getSurfaceById(surfaceId));
	let surfaceRead = $derived(surface ? getSurfaceReadModel(surface.surface_id) : undefined);
	let canViewSurface = $derived(surface ? hasPermissionValue(getUser(), surface.required_permission) : false);
	let canRenderSurface = $derived(
		surface && canViewSurface
			? shouldUseSurfaceRoute(
					getSurfaceRuntimeStatus().active,
					[surface],
					surfaceRead ? { [surface.surface_id]: surfaceRead } : {}
				)
			: false
	);
	let isPendingSurfaceRead = $derived(
		surface
			? isSurfaceTabPending({
					rolloutActive: getSurfaceRuntimeStatus().active,
					activeTab: surface.surface_id,
					slotSurfaces: [surface],
					readBySurface: surfaceRead ? { [surface.surface_id]: surfaceRead } : {},
					isReadRequested: getSurfaceReadRequested(surface.surface_id),
					isReadLoading: getSurfaceReadLoading(surface.surface_id)
				})
			: false
	);
	let pageTitle = $derived(surface?.label ?? legacyExtension?.label ?? 'Extension');

	$effect(() => {
		if (!getSurfaceRuntimeStatus().active || !surface || !canViewSurface) {
			return;
		}
		void loadSurfaceReadModels([surface.surface_id]);
	});
</script>

<svelte:head>
	<title>{pageTitle} | Uptrakit</title>
</svelte:head>

{#if getSurfaceRuntimeStatus().active && !getSurfaceRegistryLoaded()}
	<p class="py-8 text-center text-surface-500">Loading...</p>
{:else if surface && !canViewSurface}
	<div class="py-8 text-center">
		<p class="text-lg font-medium">Access denied</p>
		<p class="mt-1 text-sm text-surface-500">You do not have permission to access this surface.</p>
	</div>
{:else if isPendingSurfaceRead}
	<p class="py-8 text-center text-surface-500">Loading...</p>
{:else if canRenderSurface && surface}
	<div class="space-y-6">
		<h1 class="h1">{surface.label}</h1>
		<SurfaceReadPanel {surface} read={surfaceRead} />
	</div>
{:else if !getExtensionsLoaded()}
	<p class="py-8 text-center text-surface-500">Loading...</p>
{:else if !legacyExtension}
	<div class="py-8 text-center">
		<p class="text-lg font-medium">Extension not found</p>
		<p class="mt-1 text-sm text-surface-500">The requested extension or surface is not available.</p>
	</div>
{:else if !canViewLegacy}
	<div class="py-8 text-center">
		<p class="text-lg font-medium">Access denied</p>
		<p class="mt-1 text-sm text-surface-500">You do not have permission to access this extension.</p>
	</div>
{:else}
	<div class="space-y-6">
		<h1 class="h1">{legacyExtension.label}</h1>
		<ExtensionTabContent extension={legacyExtension} />
	</div>
{/if}
