<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { listSystemEnrollmentTokens, createSystemEnrollmentToken, revokeSystemEnrollmentToken } from '$lib/api';
	import type {
		SystemEnrollmentTokenResponse,
		SystemEnrollmentTokenCreatedResponse,
		PaginatedResponse,
		CreateSystemEnrollmentTokenRequest
	} from '$lib/types';
	import { copyToClipboard, formatDate } from '$lib/utils';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import {
		Callout,
		DataTable,
		FormFieldRow,
		ModalShell,
		SectionCard,
		StatusBadge,
		TableFooterBar,
		type DataTableColumn,
		type StatusBadgeTone
	} from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let tokens: PaginatedResponse<SystemEnrollmentTokenResponse> | null = $state(null);
	let currentPage: number = $state(1);
	let loading: boolean = $state(false);
	let loadError: string | null = $state(null);
	let showCreateDialog: boolean = $state(false);
	let createdToken: SystemEnrollmentTokenCreatedResponse | null = $state(null);
	let copied: boolean = $state(false);
	let confirmRevokeId: string | null = $state(null);
	let copyResetTimeout: ReturnType<typeof setTimeout> | null = $state(null);

	// Create form fields
	let newName: string = $state('');
	let newMaxUses: string = $state('');
	let newExpiresIn: string = $state('');
	let createNameError: string = $state('');
	let creating: boolean = $state(false);

	onMount(() => {
		void loadTokens();
	});

	onDestroy(() => {
		if (copyResetTimeout) clearTimeout(copyResetTimeout);
	});

	async function loadTokens(page = 1) {
		currentPage = page;
		loading = true;
		loadError = null;
		try {
			tokens = await listSystemEnrollmentTokens({ page });
			currentPage = tokens.page;
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load system enrollment tokens';
			onError(loadError);
		} finally {
			loading = false;
		}
	}

	async function handleCreate() {
		const trimmedName = newName.trim();
		if (!trimmedName) {
			createNameError = 'Name is required.';
			return;
		}
		createNameError = '';
		creating = true;
		try {
			const data: CreateSystemEnrollmentTokenRequest = {
				name: trimmedName
			};
			if (newMaxUses.trim()) {
				const parsed = parseInt(newMaxUses, 10);
				if (!isNaN(parsed) && parsed > 0) {
					data.max_uses = parsed;
				}
			}
			if (newExpiresIn.trim()) {
				const parsed = parseInt(newExpiresIn, 10);
				if (!isNaN(parsed) && parsed > 0) {
					data.expires_in_seconds = parsed;
				}
			}
			createdToken = await createSystemEnrollmentToken(data);
			showCreateDialog = false;
			resetForm();
			onSuccess('System enrollment token created.');
			await loadTokens();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to create system enrollment token');
		} finally {
			creating = false;
		}
	}

	async function handleRevoke(id: string) {
		try {
			await revokeSystemEnrollmentToken(id);
			onSuccess('System enrollment token revoked.');
			createdToken = null;
			await loadTokens();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to revoke system enrollment token');
		}
	}

	async function handleCopy() {
		if (createdToken && (await copyToClipboard(createdToken.token))) {
			copied = true;
			if (copyResetTimeout) clearTimeout(copyResetTimeout);
			copyResetTimeout = setTimeout(() => {
				copied = false;
				copyResetTimeout = null;
			}, 2000);
		}
	}

	function resetForm() {
		newName = '';
		newMaxUses = '';
		newExpiresIn = '';
		createNameError = '';
	}

	function formatUsage(currentUses: number, maxUses: number | null): string {
		if (maxUses == null) return `${currentUses} / unlimited`;
		return `${currentUses} / ${maxUses}`;
	}

	function tokenStatus(token: SystemEnrollmentTokenResponse): string {
		if (token.revoked_at) return 'revoked';
		if (token.expires_at && new Date(token.expires_at) < new Date()) return 'expired';
		if (token.max_uses != null && token.current_uses >= token.max_uses) return 'exhausted';
		return 'active';
	}

	function statusTone(status: string): StatusBadgeTone {
		switch (status) {
			case 'active':
				return 'success';
			case 'revoked':
				return 'danger';
			case 'expired':
			case 'exhausted':
				return 'neutral';
			default:
				return 'neutral';
		}
	}

	function handlePageChange(page: number) {
		void loadTokens(page);
	}

	const tokenColumns: DataTableColumn[] = [
		{ key: 'name', label: 'Name' },
		{ key: 'usage', label: 'Usage' },
		{ key: 'expires', label: 'Expires' },
		{ key: 'status', label: 'Status' },
		{ key: 'created', label: 'Created' },
		{ key: 'actions', label: 'Actions' }
	];

	const loadingSkeletonRows = [0, 1, 2, 3, 4];
	const showKnownShapeLoading = $derived(loading || (tokens === null && !loadError));
</script>

<SectionCard title="System Enrollment Tokens">
	<div class="mb-4 flex items-center justify-between">
		<div class="flex gap-2">
			{#if tokens === null}
				<Button variant="primary" {loading} disabled={loading} onclick={() => void loadTokens(1)}>Load Tokens</Button>
			{:else}
				<Button variant="secondary" {loading} disabled={loading} onclick={() => void loadTokens(currentPage)}>
					Refresh
				</Button>
			{/if}
			<Button
				variant="primary"
				onclick={() => {
					showCreateDialog = true;
				}}>Create Token</Button
			>
		</div>
	</div>

	<p class="mb-4 text-sm text-[var(--text-secondary)]">
		Manage system enrollment tokens for infrastructure services such as the external scheduler and MQTT bridge. Services
		that present a valid token are automatically approved on enrollment. Tokens are shown only once at creation — store
		them securely.
	</p>

	{#if createdToken}
		<Callout tone="success" title="Token created — copy it now, it will not be shown again">
			<div class="mt-2 flex items-start gap-2">
				<code class="flex-1 break-all">{createdToken.token}</code>
				<Button variant="ghost" size="sm" class="flex-shrink-0" onclick={handleCopy}>
					{copied ? 'Copied!' : 'Copy'}
				</Button>
			</div>
		</Callout>
	{/if}

	{#if showCreateDialog}
		<ModalShell
			title="Create System Enrollment Token"
			onclose={() => {
				showCreateDialog = false;
				resetForm();
			}}
		>
			<div class="space-y-4">
				<FormFieldRow label="Name" inputId="system-enrollment-token-name" required error={createNameError || undefined}>
					<Input
						id="system-enrollment-token-name"
						type="text"
						bind:value={newName}
						placeholder="e.g. MQTT Bridge Token"
						error={createNameError || undefined}
						oninput={() => {
							createNameError = '';
						}}
					/>
				</FormFieldRow>

				<FormFieldRow label="Max Uses" inputId="system-enrollment-token-max-uses">
					<Input
						id="system-enrollment-token-max-uses"
						type="number"
						bind:value={newMaxUses}
						placeholder="Unlimited"
						min={1}
					/>
				</FormFieldRow>

				<FormFieldRow
					label="Expires In (seconds)"
					inputId="system-enrollment-token-expires"
					hint="e.g. 86400 = 24 hours, 604800 = 7 days"
				>
					<Input
						id="system-enrollment-token-expires"
						type="number"
						bind:value={newExpiresIn}
						placeholder="Never"
						min={60}
					/>
				</FormFieldRow>
			</div>
			{#snippet footer()}
				<Button
					variant="secondary"
					onclick={() => {
						showCreateDialog = false;
						resetForm();
					}}>Cancel</Button
				>
				<Button variant="primary" loading={creating} onclick={handleCreate}>Create</Button>
			{/snippet}
		</ModalShell>
	{/if}

	{#if showKnownShapeLoading}
		<div
			class="overflow-hidden rounded-panel border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
			data-ui="known-shape-table-loading"
			aria-busy="true"
		>
			<div class="overflow-x-auto">
				<table class="min-w-full border-collapse text-table-body">
					<caption class="sr-only">Loading system enrollment tokens</caption>
					<thead>
						<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
							{#each tokenColumns as column (column.key)}
								<th
									class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								>
									{column.label}
								</th>
							{/each}
						</tr>
					</thead>
					<tbody>
						{#each loadingSkeletonRows as rowIndex (rowIndex)}
							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-24 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-20 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-24 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-16 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-28 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="table-cell-pad"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-16 animate-pulse rounded-card bg-[var(--bg-raised)]"
									></div></td
								>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		</div>
	{:else}
		<DataTable
			columns={tokenColumns}
			rows={(tokens?.items ?? []) as unknown as Record<string, unknown>[]}
			loading={false}
			error={loadError}
			emptyTitle="No system enrollment tokens configured."
			emptyDescription="Create a token for infrastructure services such as scheduler and MQTT bridge."
			rowKey={(row) => (row as unknown as SystemEnrollmentTokenResponse).id}
		>
			{#snippet row(rowValue, _index)}
				{@const token = rowValue as unknown as SystemEnrollmentTokenResponse}
				{@const status = tokenStatus(token)}
				<tr class="border-b border-[var(--border-subtle)] last:border-b-0 text-table-body even:bg-[var(--bg-raised)]">
					<td class="table-cell-pad">{token.name}</td>
					<td class="table-cell-pad">{formatUsage(token.current_uses, token.max_uses)}</td>
					<td class="table-cell-pad">{token.expires_at ? formatDate(token.expires_at) : 'never'}</td>
					<td class="table-cell-pad"><StatusBadge tone={statusTone(status)} label={status} /></td>
					<td class="table-cell-pad">{formatDate(token.created_at)}</td>
					<td class="table-cell-pad">
						{#if status === 'active'}
							<Button variant="danger" size="sm" onclick={() => (confirmRevokeId = token.id)}>Revoke</Button>
						{/if}
					</td>
				</tr>
			{/snippet}
			{#snippet footer()}
				{#if tokens && tokens.total_pages > 1}
					<TableFooterBar
						total={tokens.total}
						currentPage={tokens.page}
						totalPages={tokens.total_pages}
						onPageChange={handlePageChange}
					/>
				{/if}
			{/snippet}
		</DataTable>
	{/if}
</SectionCard>

{#if confirmRevokeId}
	<ConfirmDialog
		title="Revoke System Enrollment Token"
		messagePrefix="Are you sure you want to revoke"
		entityName="this system enrollment token"
		confirmLabel="Revoke"
		onconfirm={() => {
			const tokenId = confirmRevokeId;
			confirmRevokeId = null;
			if (tokenId) void handleRevoke(tokenId);
		}}
		oncancel={() => {
			confirmRevokeId = null;
		}}
	/>
{/if}
