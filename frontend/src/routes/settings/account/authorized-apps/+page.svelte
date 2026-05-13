<script lang="ts">
	import { onMount } from 'svelte';
	import { ShieldCheck } from 'lucide-svelte';
	import { listMyConsents, revokeMyConsent } from '$lib/api/oauth';
	import type { OAuthConsent } from '$lib/api/oauth';
	import { Callout, DataTable, EmptyState, PageShell, SectionCard, type DataTableColumn } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	let consents: OAuthConsent[] = $state([]);
	let loading: boolean = $state(true);
	let loadError: string | null = $state(null);
	let revoking: string | null = $state(null);
	let revokeError: string | null = $state(null);

	const columns: DataTableColumn[] = [
		{ key: 'client_name', label: 'Name', mobileTitle: true },
		{ key: 'scopes', label: 'Scopes' },
		{ key: 'granted', label: 'Granted' },
		{ key: 'last_used', label: 'Last used' },
		{ key: 'actions', label: '' }
	];

	onMount(() => {
		void loadConsents();
	});

	async function loadConsents() {
		loading = true;
		loadError = null;
		try {
			consents = await listMyConsents();
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load authorized applications';
		} finally {
			loading = false;
		}
	}

	async function handleRevoke(id: string) {
		revoking = id;
		revokeError = null;
		try {
			await revokeMyConsent(id);
			consents = consents.filter((c) => c.id !== id);
		} catch (e) {
			revokeError = e instanceof Error ? e.message : 'Failed to revoke consent';
		} finally {
			revoking = null;
		}
	}

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleDateString();
	}
</script>

<PageShell title="Authorized applications">
	{#if revokeError}
		<Callout tone="danger" message={revokeError} />
	{/if}

	<SectionCard>
		{#if loading}
			<p class="py-8 text-center text-sm text-[var(--text-secondary)]">Loading...</p>
		{:else if loadError}
			<Callout tone="danger" title="Unable to load data" message={loadError} />
		{:else if consents.length === 0}
			<EmptyState
				icon={ShieldCheck}
				title="No authorized applications"
				description="Applications you authorize via OAuth will appear here. You can revoke access at any time."
			/>
		{:else}
			<DataTable
				{columns}
				rows={consents as unknown as Record<string, unknown>[]}
				rowKey={(row) => (row as unknown as OAuthConsent).id}
			>
				{#snippet row(rowValue, _index)}
					{@const consent = rowValue as unknown as OAuthConsent}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						<td class="table-cell-pad text-[var(--text-primary)]">{consent.client_name}</td>
						<td class="table-cell-pad text-[var(--text-primary)]">{consent.scopes.join(', ')}</td>
						<td class="table-cell-pad text-[var(--text-primary)]">{formatDate(consent.granted_at)}</td>
						<td class="table-cell-pad text-[var(--text-primary)]">
							{consent.last_used_at ? formatDate(consent.last_used_at) : 'Never'}
						</td>
						<td class="table-cell-pad">
							<Button
								variant="danger"
								size="sm"
								loading={revoking === consent.id}
								disabled={revoking !== null}
								onclick={() => void handleRevoke(consent.id)}
							>
								Revoke
							</Button>
						</td>
					</tr>
				{/snippet}
			</DataTable>
		{/if}
	</SectionCard>
</PageShell>
