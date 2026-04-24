<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { confirmEmailChange } from '$lib/api';
	import { Callout } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import { PUBLIC_ENTRY_FORM_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
	import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';

	type ConfirmState = 'loading' | 'success' | 'error';
	let status: ConfirmState = $state('loading');
	let errorMessage = $state('');

	onMount(async () => {
		const token = page.url.searchParams.get('token');
		if (!token) {
			errorMessage = 'Invalid confirmation link.';
			status = 'error';
			return;
		}
		try {
			await confirmEmailChange(token);
			status = 'success';
			setTimeout(() => goto('/login'), 2000);
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : 'Failed to confirm email change.';
			status = 'error';
		}
	});
</script>

<PublicEntryShell title="Confirm email change">
	<div class={PUBLIC_ENTRY_FORM_CLASS}>
		{#if status === 'loading'}
			<Button variant="primary" loading={true} disabled>Confirming…</Button>
		{:else if status === 'success'}
			<Callout tone="success">Your email address has been updated. Please log in with your new address.</Callout>
			<Button variant="primary" onclick={() => goto('/login')}>Go to login</Button>
		{:else}
			<Callout tone="danger">{errorMessage}</Callout>
			<Button variant="ghost" href="/profile">Back to profile</Button>
		{/if}
	</div>
</PublicEntryShell>
