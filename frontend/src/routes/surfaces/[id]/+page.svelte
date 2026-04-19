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
		loadSurfaceReadModels
	} from '$lib/surfaces/registry.svelte';
	import { isSurfaceTabPending } from '$lib/surfaces/read-model';
	import { hasPermissionValue } from '$lib/types';
	import { Callout, PageShell, SectionCard } from '$lib/components/ui';

	let surfaceId = $derived(page.params.id as string);
	let surface = $derived(getSurfaceById(surfaceId));
	let surfaceRead = $derived(surface ? getSurfaceReadModel(surface.surface_id) : undefined);
	let isReadRequested = $derived(surface ? getSurfaceReadRequested(surface.surface_id) : false);
	let isReadLoading = $derived(surface ? getSurfaceReadLoading(surface.surface_id) : false);
	let canViewSurface = $derived(surface ? hasPermissionValue(getUser(), surface.required_permission) : false);
	let isPendingSurfaceRead = $derived(
		surface
			? isSurfaceTabPending({
					activeTab: surface.surface_id,
					slotSurfaces: [surface],
					readBySurface: surfaceRead ? { [surface.surface_id]: surfaceRead } : {},
					isReadRequested,
					isReadLoading
				})
			: false
	);
	let pageTitle = $derived(surface?.label ?? 'Surface');

	$effect(() => {
		if (!surface || !canViewSurface) {
			return;
		}
		if (isReadRequested || isReadLoading) {
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
	{:else if surface}
		<div data-parity-region="surface.page">
			<SectionCard>
				{#if !canViewSurface}
					<Callout tone="danger" title="Access denied" message="You do not have permission to access this surface." />
				{:else if isPendingSurfaceRead}
					<p class="py-8 text-center text-surface-500">Loading...</p>
				{:else}
					<SurfaceReadPanel {surface} read={surfaceRead} />
				{/if}
			</SectionCard>
		</div>
	{:else if !surface}
		<Callout tone="warning" title="Surface not found" message="The requested surface is not available." />
	{/if}
</PageShell>
