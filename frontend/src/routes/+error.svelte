<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { Callout } from '$lib/components/ui';
	import PublicEntryShell, { PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
</script>

<PublicEntryShell eyebrow="Error" title="Something went wrong" subtitle="The requested page could not be loaded.">
	<Callout
		tone="danger"
		title={`Error ${page.status}`}
		message={page.error?.message ?? 'An unexpected error occurred.'}
	/>

	{#snippet footer()}
		<button class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS} onclick={() => goto('/')}>Go to Home</button>
	{/snippet}
</PublicEntryShell>
