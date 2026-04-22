<script lang="ts">
	import SurfaceInteractionButton from '$lib/components/surfaces/SurfaceInteractionButton.svelte';
	import type { InteractionDescriptor } from '$lib/surfaces/contract';

	const primaryInteraction: InteractionDescriptor = {
		interaction_id: 'demo.primary',
		kind: 'mutation_action',
		label: 'Primary Action',
		transport: { mode: 'controller_local' }
	};

	const dangerInteraction: InteractionDescriptor = {
		interaction_id: 'demo.danger',
		kind: 'mutation_action',
		label: 'Danger Action',
		transport: { mode: 'controller_local' },
		confirmation: {
			title: 'Confirm',
			message: 'Are you sure?',
			severity: 'danger'
		}
	};

	const formInteraction: InteractionDescriptor = {
		interaction_id: 'demo.form',
		kind: 'form_submit',
		label: 'Form Action',
		transport: { mode: 'controller_local' },
		form_ui: {
			fields: [{ key: 'name', label: 'Name', field_type: 'text', required: false }]
		}
	};
</script>

<main class="flex flex-col gap-6 p-6" data-testid="surface-preview-root">
	<section data-testid="surface-primary">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">SurfaceInteractionButton — primary</h2>
		<SurfaceInteractionButton
			surfaceId="demo.surface"
			interaction={primaryInteraction}
			interactions={[primaryInteraction]}
		/>
	</section>

	<section data-testid="surface-danger">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">SurfaceInteractionButton — danger</h2>
		<SurfaceInteractionButton
			surfaceId="demo.surface"
			interaction={dangerInteraction}
			interactions={[dangerInteraction]}
		/>
	</section>

	<section data-testid="surface-form">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">SurfaceInteractionButton — form</h2>
		<SurfaceInteractionButton surfaceId="demo.surface" interaction={formInteraction} interactions={[formInteraction]} />
	</section>

	<section data-testid="surface-sm">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">SurfaceInteractionButton — size=sm</h2>
		<SurfaceInteractionButton
			surfaceId="demo.surface"
			interaction={primaryInteraction}
			interactions={[primaryInteraction]}
			size="sm"
		/>
	</section>
</main>
