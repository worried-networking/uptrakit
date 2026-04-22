<script lang="ts">
	import Button from '$lib/components/Button.svelte';
	import UpdateAllButton from '$lib/components/UpdateAllButton.svelte';
	import type { ButtonVariant, ButtonSize } from '$lib/components/Button.svelte';

	const VARIANTS: ButtonVariant[] = ['primary', 'ghost', 'danger'];
	const SIZES: ButtonSize[] = ['md', 'sm'];

	function noop() {}
</script>

<main class="flex flex-col gap-6 p-6" data-testid="button-preview-root">
	<section data-testid="button-variants">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Button — variants × sizes</h2>
		<div class="grid grid-cols-2 gap-3" style="width: 420px;">
			{#each VARIANTS as variant (variant)}
				{#each SIZES as size (size)}
					<div class="flex items-center gap-2" data-testid="button-cell-{variant}-{size}">
						<Button {variant} {size} onclick={noop}>Label</Button>
						<span class="text-[9px] uppercase text-[var(--text-muted)]">
							{variant}/{size}
						</span>
					</div>
				{/each}
			{/each}
		</div>
	</section>

	<section data-testid="button-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Button — disabled + loading</h2>
		<div class="flex flex-wrap gap-3">
			<Button variant="primary" disabled onclick={noop}>Disabled</Button>
			<Button variant="primary" loading onclick={noop}>Loading</Button>
			<Button variant="ghost" disabled onclick={noop}>Ghost disabled</Button>
			<Button variant="danger" loading onclick={noop}>Danger loading</Button>
		</div>
	</section>

	<section data-testid="button-link">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Button — link branch</h2>
		<div class="flex flex-wrap gap-3">
			<Button variant="ghost" href="/login">Href ghost</Button>
			<Button variant="primary" href="/register">Href primary</Button>
			<Button variant="primary" href="/disabled" disabled>Href disabled</Button>
		</div>
	</section>

	<section data-testid="updateall-states">
		<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">UpdateAllButton</h2>
		<div class="flex flex-wrap gap-3">
			<UpdateAllButton state="idle" onclick={noop} />
			<UpdateAllButton state="idle" count={3} onclick={noop} />
			<UpdateAllButton state="dim" ariaLabel="No updates available" onclick={noop} />
		</div>
	</section>
</main>
