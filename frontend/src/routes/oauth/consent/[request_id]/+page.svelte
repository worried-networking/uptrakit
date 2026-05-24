<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { CheckCircle } from 'lucide-svelte';
	import { Callout } from '$lib/components/ui';
	import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';
	import ConsentPrompt from '$lib/components/ConsentPrompt.svelte';
	import type { ConsentPromptTrust } from '$lib/components/ConsentPrompt.svelte';
	import Link from '$lib/components/Link.svelte';
	import { getConsentDetails, approveConsent, denyConsent, type ConsentDetails } from '$lib/api/oauth';
	import { getUser } from '$lib/auth.svelte';

	let details = $state<ConsentDetails | null>(null);
	let loadError = $state<string | null>(null);
	let submitting = $state(false);

	const requestId = $derived(page.params.request_id ?? '');
	const pageTitle = $derived(details ? `${details.client_name} wants access` : 'Authorize Access');

	const LOCAL_REDIRECT_HOSTS = ['localhost', '127.0.0.1', '[::1]'];
	const isLocalRedirect = $derived(details ? LOCAL_REDIRECT_HOSTS.includes(details.redirect_uri_host) : false);

	onMount(async () => {
		try {
			details = await getConsentDetails(requestId);
		} catch (e) {
			loadError = e instanceof Error ? e.message : String(e);
		}
	});

	function clientTrust(d: ConsentDetails): ConsentPromptTrust {
		if (d.trusted_at === null) return 'unverified';
		if (d.created_via === 'dcr') return 'dcr';
		if (d.created_via === 'cimd_cache') return 'open-metadata';
		return 'manual';
	}

	function scopeDescription(scope: string): string {
		if (scope === 'mcp:read') return 'Read your uptrakit data (update history, host info, account profile)';
		if (scope === 'mcp:write') return 'Trigger software updates on your behalf';
		return scope;
	}

	async function handleAllow() {
		if (!details) return;
		submitting = true;
		try {
			const resp = await approveConsent(requestId);
			window.location.href = resp.redirect_to;
		} catch (e) {
			submitting = false;
			loadError = e instanceof Error ? e.message : String(e);
		}
	}

	async function handleDeny() {
		if (!details) return;
		submitting = true;
		try {
			const resp = await denyConsent(requestId);
			window.location.href = resp.redirect_to;
		} catch (e) {
			submitting = false;
			loadError = e instanceof Error ? e.message : String(e);
		}
	}
</script>

<PublicEntryShell eyebrow="Authorize Access" title={pageTitle}>
	{#if loadError}
		<Callout tone="danger" message={loadError} />
	{:else if details !== null}
		<ConsentPrompt
			trust={clientTrust(details)}
			approveDisabled={submitting}
			approving={submitting}
			onApprove={handleAllow}
			onDeny={handleDeny}
		>
			{#if details.metadata_change_diff}
				<Callout
					tone="warning"
					message="This client's published metadata has changed since you last authorized it. Review before continuing."
				/>
			{/if}

			{#if isLocalRedirect}
				<Callout
					tone="warning"
					message="This client will receive credentials at a local address. Make sure it is running on this machine."
				/>
			{/if}

			<ul class="space-y-1">
				{#each details.scopes as scope (scope)}
					<li class="flex items-start gap-2 text-sm text-[var(--text-secondary)]">
						<CheckCircle size={14} class="mt-0.5 shrink-0 text-[var(--color-success)]" aria-hidden="true" />
						{scopeDescription(scope)}
					</li>
				{/each}
			</ul>

			<Callout
				tone="info"
				message="{details.client_name} will act using your existing permissions — it cannot do anything you cannot already do."
			/>
		</ConsentPrompt>
	{:else}
		<Callout tone="info" message="Loading…" />
	{/if}

	{#snippet footer()}
		<p class="text-sm text-[var(--text-secondary)]">
			Signed in as {getUser()?.email ?? ''}
			· <Link href="/login?_auth_context=oauth">Switch account</Link>
		</p>
	{/snippet}
</PublicEntryShell>
