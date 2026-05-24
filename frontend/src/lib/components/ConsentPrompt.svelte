<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export type ConsentPromptTrust = 'verified' | 'unverified' | 'dcr' | 'open-metadata' | 'manual';
</script>

<script lang="ts">
	import { Callout } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	let {
		trust,
		approveDisabled = false,
		approving,
		onApprove,
		onDeny,
		children
	}: {
		trust: ConsentPromptTrust;
		approveDisabled?: boolean;
		approving: boolean;
		onApprove: () => void;
		onDeny: () => void;
		children?: Snippet;
	} = $props();
</script>

<div class="space-y-4" data-ui="consent-prompt">
	{#if trust === 'unverified'}
		<Callout tone="danger" message="This client has not been verified. Proceed only if you trust it." />
	{:else if trust === 'dcr'}
		<Callout tone="warning" message="This client was recently registered and has not been reviewed." />
	{/if}

	{@render children?.()}

	<div class="flex justify-end gap-2">
		<Button variant="secondary" disabled={approving} onclick={onDeny}>Deny</Button>
		<Button variant="primary" loading={approving} disabled={approveDisabled || approving} onclick={onApprove}>
			Approve
		</Button>
	</div>
</div>
