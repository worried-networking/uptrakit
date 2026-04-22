<script lang="ts">
	import { page } from '$app/state';
	import { Callout } from '$lib/components/ui';
	import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';
	import Button from '$lib/components/Button.svelte';
</script>

<PublicEntryShell eyebrow="Error" title="Something went wrong" subtitle="The requested page could not be loaded.">
	<Callout
		tone="danger"
		title={`Error ${page.status}`}
		message={page.error?.message ?? 'An unexpected error occurred.'}
	/>

	{#snippet footer()}
		<Button variant="primary" href="/" class="w-full justify-center">Go to Home</Button>
	{/snippet}
</PublicEntryShell>
