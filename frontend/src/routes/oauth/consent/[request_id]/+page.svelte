<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { BadgeCheck, ExternalLink, AlertTriangle } from 'lucide-svelte';
	import { PageShell, SectionCard, Callout, StatusBadge } from '$lib/components/ui';
	import type { StatusBadgeTone } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import { getConsentDetails, approveConsent, denyConsent, type ConsentDetails } from '$lib/api/oauth';
	import { getUser } from '$lib/auth.svelte';

	let details = $state<ConsentDetails | null>(null);
	let loadError = $state<string | null>(null);
	let typedConfirmation = $state('');
	let submitting = $state(false);

	const requestId = $derived(page.params.request_id ?? '');

	onMount(async () => {
		try {
			details = await getConsentDetails(requestId);
		} catch (e) {
			loadError = e instanceof Error ? e.message : String(e);
		}
	});

	const allowDisabled = $derived(
		submitting ||
			!details ||
			(details.requires_typed_confirmation &&
				typedConfirmation.trim().toLowerCase() !== details.typed_confirmation_value.toLowerCase())
	);

	async function handleAllow() {
		if (!details) return;
		submitting = true;
		try {
			const resp = await approveConsent(requestId, details.requires_typed_confirmation ? typedConfirmation : null);
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

	interface ClientBadge {
		tone: StatusBadgeTone;
		label: string;
	}

	function clientBadge(d: ConsentDetails): ClientBadge {
		if (d.trusted_at === null) {
			return { tone: 'danger', label: 'Unverified client' };
		}
		switch (d.created_via) {
			case 'dcr':
				return { tone: 'warning', label: 'Newly registered (DCR)' };
			case 'cimd_cache':
				return { tone: 'neutral', label: 'Open client metadata' };
			case 'manual':
				return { tone: 'neutral', label: 'Manually registered' };
			default: {
				const _exhaustive: never = d.created_via;
				return { tone: 'neutral', label: String(_exhaustive) };
			}
		}
	}

	const LOCAL_REDIRECT_HOSTS = ['localhost', '127.0.0.1', '[::1]'];

	function scopeDescription(scope: string): string {
		if (scope === 'mcp:read') return 'Read your uptrakit data (update history, host info, account profile)';
		if (scope === 'mcp:write') return 'Trigger software updates on your behalf';
		return scope;
	}

	const clientBadgeValue = $derived(details ? clientBadge(details) : { tone: 'neutral' as StatusBadgeTone, label: '' });
	const isLocalRedirect = $derived(details ? LOCAL_REDIRECT_HOSTS.includes(details.redirect_uri_host) : false);
	const pageTitle = $derived(details ? `${details.client_name} wants access to your uptrakit account` : 'Loading…');
</script>

<PageShell eyebrow="Authorize Access" title={pageTitle}>
	{#if loadError}
		<Callout tone="danger" message={loadError} />
	{:else if details !== null}
		<SectionCard title="Client">
			<div class="flex items-center gap-3">
				<BadgeCheck size={20} class="text-[var(--text-muted)]" aria-hidden="true" />
				<div class="flex-1">
					<div class="text-page-title font-bold text-[var(--text-primary)]">
						{details.client_name}
					</div>
					{#if details.client_uri}
						<a
							href={details.client_uri}
							target="_blank"
							rel="noopener noreferrer"
							class="inline-flex items-center gap-1 text-sm text-[var(--text-secondary)]"
						>
							{details.client_uri}
							<ExternalLink size={14} aria-hidden="true" />
						</a>
					{/if}
				</div>
				<StatusBadge tone={clientBadgeValue.tone} label={clientBadgeValue.label} />
			</div>
		</SectionCard>

		<SectionCard title="Redirect URI">
			<p class="font-mono text-sm text-[var(--text-primary)]">{details.redirect_uri_host}</p>
			{#if isLocalRedirect}
				<div class="mt-3">
					<Callout tone="warning">
						<div class="flex items-center gap-2">
							<AlertTriangle size={16} aria-hidden="true" />
							<span>
								This client is asking to receive credentials at a local-only address. Make sure you are running it on
								this machine right now.
							</span>
						</div>
					</Callout>
				</div>
			{/if}
		</SectionCard>

		{#if details.metadata_change_diff}
			<SectionCard title="Metadata change notice">
				<Callout
					tone="warning"
					message="This client's published metadata has changed since you last authorized it. Review the new details before continuing."
				/>
			</SectionCard>
		{/if}

		<SectionCard title="Permissions requested">
			<ul class="space-y-2">
				{#each details.scopes as scope (scope)}
					<li class="flex items-center gap-2">
						<StatusBadge tone="neutral" label={scope} />
						<span class="text-sm text-[var(--text-secondary)]">{scopeDescription(scope)}</span>
					</li>
				{/each}
			</ul>
			<div class="mt-3">
				<Callout
					tone="info"
					message="{details.client_name} will act using your existing permissions — it cannot do anything you cannot already do."
				/>
			</div>
		</SectionCard>

		<SectionCard title="Signed in as">
			<div class="flex items-start justify-between gap-4">
				<span class="text-sm text-[var(--text-primary)]">{getUser()?.email ?? ''}</span>
				<a href="/login?_auth_context=oauth" class="text-sm text-[var(--text-secondary)] underline">
					Not you? Sign in as someone else
				</a>
			</div>
		</SectionCard>

		{#if details.requires_typed_confirmation}
			<SectionCard title="Confirm redirect URI">
				<p class="mb-2 text-sm text-[var(--text-secondary)]">
					Type the redirect hostname below to confirm you have verified it matches what you expect.
				</p>
				<label class="block">
					<span class="mb-1 block text-xs text-[var(--text-muted)]">
						Expected: {details.typed_confirmation_value}
					</span>
					<input
						type="text"
						bind:value={typedConfirmation}
						class="h-8 w-full"
						autocomplete="off"
						data-ui="typed-confirmation-input"
					/>
				</label>
			</SectionCard>
		{/if}

		<div class="flex justify-end gap-2 pt-2">
			<Button variant="secondary" onclick={handleDeny} disabled={submitting}>Deny</Button>
			<Button variant="primary" onclick={handleAllow} disabled={allowDisabled}>Allow access</Button>
		</div>
	{:else}
		<Callout tone="info" message="Loading…" />
	{/if}
</PageShell>
