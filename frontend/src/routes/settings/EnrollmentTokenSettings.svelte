<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { listEnrollmentTokens, createEnrollmentToken, revokeEnrollmentToken } from '$lib/api';
	import type {
		EnrollmentTokenResponse,
		EnrollmentTokenCreatedResponse,
		EnrollmentTokensSummary,
		PaginatedResponse,
		CreateEnrollmentTokenRequest
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
		type DataTableColumn
	} from '$lib/components/ui';

	let {
		summary,
		onSuccess,
		onError
	}: {
		summary: EnrollmentTokensSummary | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let tokens: PaginatedResponse<EnrollmentTokenResponse> | null = $state(null);
	let currentPage: number = $state(1);
	let loading: boolean = $state(false);
	let loadError: string | null = $state(null);
	let showCreateDialog: boolean = $state(false);
	let createdToken: EnrollmentTokenCreatedResponse | null = $state(null);
	let copied: boolean = $state(false);
	let confirmRevokeId: string | null = $state(null);
	let copyResetTimeout: ReturnType<typeof setTimeout> | null = $state(null);

	// Create form fields
	let newName: string = $state('');
	let newCapabilities: string = $state('');
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
			tokens = await listEnrollmentTokens(page);
			currentPage = tokens.page;
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load enrollment tokens';
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
			const data: CreateEnrollmentTokenRequest = {
				name: trimmedName
			};
			if (newCapabilities.trim()) {
				data.allowed_capabilities = newCapabilities
					.split(',')
					.map((c) => c.trim())
					.filter((c) => c.length > 0);
			}
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
			createdToken = await createEnrollmentToken(data);
			showCreateDialog = false;
			resetForm();
			onSuccess('Enrollment token created.');
			await loadTokens();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to create enrollment token');
		} finally {
			creating = false;
		}
	}

	async function handleRevoke(id: string) {
		try {
			await revokeEnrollmentToken(id);
			onSuccess('Enrollment token revoked.');
			createdToken = null;
			await loadTokens();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to revoke enrollment token');
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
		newCapabilities = '';
		newMaxUses = '';
		newExpiresIn = '';
		createNameError = '';
	}

	function formatCapabilities(caps: string[] | null): string {
		if (!caps || caps.length === 0) return 'wildcard';
		return caps.join(', ');
	}

	function formatUsage(currentUses: number, maxUses: number | null): string {
		if (maxUses == null) return `${currentUses} / unlimited`;
		return `${currentUses} / ${maxUses}`;
	}

	function tokenStatus(token: EnrollmentTokenResponse): string {
		if (token.revoked_at) return 'revoked';
		if (token.expires_at && new Date(token.expires_at) < new Date()) return 'expired';
		if (token.max_uses != null && token.current_uses >= token.max_uses) return 'exhausted';
		return 'active';
	}

	function statusTone(status: string): 'success' | 'danger' | 'neutral' {
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
		{ key: 'capabilities', label: 'Capabilities' },
		{ key: 'usage', label: 'Usage' },
		{ key: 'expires', label: 'Expires' },
		{ key: 'status', label: 'Status' },
		{ key: 'created', label: 'Created' },
		{ key: 'actions', label: 'Actions' }
	];

	const loadingSkeletonRows = [0, 1, 2, 3, 4];
	const showKnownShapeLoading = $derived(loading || (tokens === null && !loadError));
</script>

<SectionCard title="Enrollment Tokens">
	<div class="mb-4 flex items-center justify-between">
		<div class="flex gap-2">
			{#if tokens === null}
				<button class="btn preset-filled-primary-500" onclick={() => void loadTokens(1)} disabled={loading}>
					{loading ? 'Loading...' : 'Load Tokens'}
				</button>
			{:else}
				<button class="btn preset-tonal" onclick={() => void loadTokens(currentPage)} disabled={loading}>
					Refresh
				</button>
			{/if}
			<button
				class="btn preset-filled-primary-500"
				onclick={() => {
					showCreateDialog = true;
				}}
			>
				Create Token
			</button>
		</div>
	</div>

	{#if summary !== undefined}
		<p class="mb-4 text-sm text-surface-600 dark:text-surface-400">
			{summary.active_count} active enrollment {summary.active_count === 1 ? 'token' : 'tokens'} configured. Tokens allow
			services to enroll automatically with approved status.
		</p>
	{/if}

	{#if createdToken}
		<Callout tone="success" title="Token created — copy it now, it will not be shown again">
			<div class="mt-2 flex items-start gap-2">
				<code class="flex-1 break-all">{createdToken.token}</code>
				<button class="btn btn-sm preset-tonal flex-shrink-0" onclick={handleCopy}>
					{copied ? 'Copied!' : 'Copy'}
				</button>
			</div>
		</Callout>
	{/if}

	{#if showCreateDialog}
		<ModalShell
			title="Create Enrollment Token"
			onclose={() => {
				showCreateDialog = false;
				resetForm();
			}}
		>
			<div class="space-y-4">
				<FormFieldRow label="Name" inputId="enrollment-token-name" required error={createNameError || undefined}>
					<input
						id="enrollment-token-name"
						class="input"
						type="text"
						bind:value={newName}
						placeholder="e.g. CI Deploy Token"
						aria-invalid={createNameError ? 'true' : undefined}
						oninput={() => {
							createNameError = '';
						}}
					/>
				</FormFieldRow>

				<FormFieldRow
					label="Allowed Capabilities"
					inputId="enrollment-token-capabilities"
					hint="Comma-separated. Leave empty for a wildcard token."
				>
					<input
						id="enrollment-token-capabilities"
						class="input"
						type="text"
						bind:value={newCapabilities}
						placeholder="e.g. software_discovery, mqtt_bridge (empty = wildcard)"
					/>
				</FormFieldRow>

				<FormFieldRow label="Max Uses" inputId="enrollment-token-max-uses">
					<input
						id="enrollment-token-max-uses"
						class="input"
						type="number"
						bind:value={newMaxUses}
						placeholder="Unlimited"
						min="1"
					/>
				</FormFieldRow>

				<FormFieldRow
					label="Expires In (seconds)"
					inputId="enrollment-token-expires"
					hint="e.g. 86400 = 24 hours, 604800 = 7 days"
				>
					<input
						id="enrollment-token-expires"
						class="input"
						type="number"
						bind:value={newExpiresIn}
						placeholder="Never"
						min="60"
					/>
				</FormFieldRow>
			</div>
			{#snippet footer()}
				<button
					class="btn preset-tonal"
					onclick={() => {
						showCreateDialog = false;
						resetForm();
					}}
				>
					Cancel
				</button>
				<button class="btn preset-filled-primary-500" onclick={handleCreate} disabled={creating}>
					{creating ? 'Creating...' : 'Create'}
				</button>
			{/snippet}
		</ModalShell>
	{/if}

	{#if showKnownShapeLoading}
		<div
			class="overflow-hidden rounded-[4px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
			data-ui="known-shape-table-loading"
			aria-busy="true"
		>
			<div class="overflow-x-auto">
				<table class="min-w-full border-collapse text-[12px]">
					<caption class="sr-only">Loading enrollment tokens</caption>
					<thead>
						<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
							{#each tokenColumns as column (column.key)}
								<th class="px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.12em]" scope="col">
									{column.label}
								</th>
							{/each}
						</tr>
					</thead>
					<tbody>
						{#each loadingSkeletonRows as rowIndex (rowIndex)}
							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
								<td class="px-4 py-3"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-24 animate-pulse rounded bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="px-4 py-3"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-40 animate-pulse rounded bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="px-4 py-3"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-20 animate-pulse rounded bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="px-4 py-3"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-24 animate-pulse rounded bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="px-4 py-3"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-16 animate-pulse rounded bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="px-4 py-3"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-28 animate-pulse rounded bg-[var(--bg-raised)]"
									></div></td
								>
								<td class="px-4 py-3"
									><div
										data-ui="loading-skeleton-cell"
										class="h-3 w-16 animate-pulse rounded bg-[var(--bg-raised)]"
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
			emptyTitle="No enrollment tokens configured."
			emptyDescription="Create a token to allow approved services to enroll automatically."
			rowKey={(row) => (row as unknown as EnrollmentTokenResponse).id}
		>
			{#snippet row(rowValue)}
				{@const token = rowValue as unknown as EnrollmentTokenResponse}
				{@const status = tokenStatus(token)}
				<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
					<td class="px-4 py-3">{token.name}</td>
					<td class="px-4 py-3">
						{#if !token.allowed_capabilities || token.allowed_capabilities.length === 0}
							<StatusBadge tone="neutral" label="wildcard" />
						{:else}
							{formatCapabilities(token.allowed_capabilities)}
						{/if}
					</td>
					<td class="px-4 py-3">{formatUsage(token.current_uses, token.max_uses)}</td>
					<td class="px-4 py-3">{token.expires_at ? formatDate(token.expires_at) : 'never'}</td>
					<td class="px-4 py-3"><StatusBadge tone={statusTone(status)} label={status} /></td>
					<td class="px-4 py-3">{formatDate(token.created_at)}</td>
					<td class="px-4 py-3">
						{#if status === 'active'}
							<button class="btn btn-sm preset-filled-error-500" onclick={() => (confirmRevokeId = token.id)}>
								Revoke
							</button>
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
		title="Revoke Enrollment Token"
		messagePrefix="Are you sure you want to revoke"
		entityName="this enrollment token"
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
