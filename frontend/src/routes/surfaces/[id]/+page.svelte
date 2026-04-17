<script lang="ts">
	import { page } from '$app/state';
	import { getUser } from '$lib/auth.svelte';
	import SurfaceReadPanel from '$lib/components/surfaces/SurfaceReadPanel.svelte';
	import {
		getSurfaceById,
		getSurfaceReadLoading,
		getSurfaceReadModel,
		getSurfaceReadRequested,
		getSurfaceRegistryLoaded,
		getSurfaceRuntimeStatus,
		loadSurfaceReadModels
	} from '$lib/surfaces/registry.svelte';
	import { isSurfaceTabPending } from '$lib/surfaces/read-model';
	import { hasPermissionValue } from '$lib/types';
	import { Callout, PageShell, SectionCard } from '$lib/components/ui';

	let surfaceId = $derived(page.params.id as string);
	let surface = $derived(getSurfaceById(surfaceId));
	let surfaceRead = $derived(surface ? getSurfaceReadModel(surface.surface_id) : undefined);
	let canViewSurface = $derived(surface ? hasPermissionValue(getUser(), surface.required_permission) : false);
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
	let pageTitle = $derived(surface?.label ?? 'Surface');

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

<PageShell title={pageTitle}>
	{#if !getSurfaceRegistryLoaded()}
		<SectionCard title={pageTitle}>
			<p class="py-8 text-center text-surface-500">Loading...</p>
		</SectionCard>
	{:else if surface && !canViewSurface}
		<Callout tone="danger" title="Access denied" message="You do not have permission to access this surface." />
	{:else if isPendingSurfaceRead}
		<SectionCard>
			<p class="py-8 text-center text-surface-500">Loading...</p>
		</SectionCard>
	{:else if surface && canViewSurface}
		<SectionCard>
			<SurfaceReadPanel {surface} read={surfaceRead} />
		</SectionCard>
	{:else if !surface}
		<Callout tone="warning" title="Surface not found" message="The requested surface is not available." />
	{/if}
</PageShell>
