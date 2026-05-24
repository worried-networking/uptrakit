<script lang="ts" module>
	import type { Snippet } from 'svelte';

	export type ConsentPromptTrust = 'verified' | 'unverified' | 'dcr' | 'open-metadata' | 'manual';
</script>

<script lang="ts">
	import { ExternalLink } from 'lucide-svelte';
	import { Callout } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	let {
		clientName,
		clientUri,
		trust,
		approveDisabled = false,
		approving,
		onApprove,
		onDeny,
		children
	}: {
		clientName: string;
		clientUri?: string | null;
		trust: ConsentPromptTrust;
		approveDisabled?: boolean;
		approving: boolean;
		onApprove: () => void;
		onDeny: () => void;
		children?: Snippet;
	} = $props();
</script>

<div class="space-y-4" data-ui="consent-prompt">
	<div>
		<p class="text-page-title font-bold text-[var(--text-primary)]">{clientName}</p>
		{#if clientUri}
			<a
				href={clientUri}
				target="_blank"
				rel="noopener noreferrer"
				class="inline-flex items-center gap-1 text-sm text-[var(--text-secondary)]"
			>
				{clientUri}
				<ExternalLink size={14} aria-hidden="true" />
			</a>
		{/if}
	</div>

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
