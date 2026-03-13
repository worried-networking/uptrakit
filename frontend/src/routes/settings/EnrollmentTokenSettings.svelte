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
	let loading: boolean = $state(false);
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
	let creating: boolean = $state(false);

	onMount(() => {
		void loadTokens();
	});

	onDestroy(() => {
		if (copyResetTimeout) clearTimeout(copyResetTimeout);
	});

	async function loadTokens() {
		loading = true;
		try {
			tokens = await listEnrollmentTokens();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to load enrollment tokens');
		} finally {
			loading = false;
		}
	}

	async function handleCreate() {
		if (!newName.trim()) {
			onError('Token name is required');
			return;
		}
		creating = true;
		try {
			const data: CreateEnrollmentTokenRequest = {
				name: newName.trim()
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

	function statusBadgeClass(status: string): string {
		switch (status) {
			case 'active':
				return 'preset-filled-success-500';
			case 'revoked':
				return 'preset-filled-error-500';
			case 'expired':
			case 'exhausted':
				return 'preset-tonal';
			default:
				return 'preset-tonal';
		}
	}
</script>

<div class="card mb-6 p-6">
	<div class="mb-4 flex items-center justify-between">
		<h2 class="h3">Enrollment Tokens</h2>
		<div class="flex gap-2">
			{#if tokens === null}
				<button class="btn preset-filled-primary-500" onclick={loadTokens} disabled={loading}>
					{loading ? 'Loading...' : 'Load Tokens'}
				</button>
			{:else}
				<button class="btn preset-tonal" onclick={loadTokens} disabled={loading}> Refresh </button>
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
		<aside class="mb-4 rounded-lg p-4 preset-filled-success-500">
			<p class="font-bold">Token created — copy it now, it will not be shown again</p>
			<div class="mt-2 flex items-start gap-2">
				<code class="flex-1 break-all">{createdToken.token}</code>
				<button class="btn btn-sm preset-tonal flex-shrink-0" onclick={handleCopy}>
					{copied ? 'Copied!' : 'Copy'}
				</button>
			</div>
		</aside>
	{/if}

	{#if showCreateDialog}
		<div class="mb-4 rounded-lg border p-4">
			<h3 class="h4 mb-3">Create Enrollment Token</h3>
			<div class="space-y-3">
				<label class="label">
					<span>Name *</span>
					<input class="input" type="text" bind:value={newName} placeholder="e.g. CI Deploy Token" />
				</label>
				<label class="label">
					<span>Allowed Capabilities</span>
					<input
						class="input"
						type="text"
						bind:value={newCapabilities}
						placeholder="e.g. software_discovery, mqtt_bridge (empty = wildcard)"
					/>
					<span class="text-xs text-surface-500">Comma-separated. Leave empty for a wildcard token.</span>
				</label>
				<label class="label">
					<span>Max Uses</span>
					<input class="input" type="number" bind:value={newMaxUses} placeholder="Unlimited" min="1" />
				</label>
				<label class="label">
					<span>Expires In (seconds)</span>
					<input class="input" type="number" bind:value={newExpiresIn} placeholder="Never" min="60" />
					<span class="text-xs text-surface-500">e.g. 86400 = 24 hours, 604800 = 7 days</span>
				</label>
				<div class="flex gap-2">
					<button class="btn preset-filled-primary-500" onclick={handleCreate} disabled={creating}>
						{creating ? 'Creating...' : 'Create'}
					</button>
					<button
						class="btn preset-tonal"
						onclick={() => {
							showCreateDialog = false;
							resetForm();
						}}
					>
						Cancel
					</button>
				</div>
			</div>
		</div>
	{/if}

	{#if loading}
		<p class="text-surface-600 dark:text-surface-400">Loading tokens...</p>
	{:else if tokens && tokens.items.length > 0}
		<div class="table-container">
			<table class="table">
				<thead>
					<tr>
						<th>Name</th>
						<th>Capabilities</th>
						<th>Usage</th>
						<th>Expires</th>
						<th>Status</th>
						<th>Created</th>
						<th></th>
					</tr>
				</thead>
				<tbody>
					{#each tokens.items as token (token.id)}
						{@const status = tokenStatus(token)}
						<tr>
							<td>{token.name}</td>
							<td>
								{#if !token.allowed_capabilities || token.allowed_capabilities.length === 0}
									<span class="badge preset-tonal text-xs">wildcard</span>
								{:else}
									{formatCapabilities(token.allowed_capabilities)}
								{/if}
							</td>
							<td>{formatUsage(token.current_uses, token.max_uses)}</td>
							<td>{token.expires_at ? formatDate(token.expires_at) : 'never'}</td>
							<td><span class="badge {statusBadgeClass(status)}">{status}</span></td>
							<td>{formatDate(token.created_at)}</td>
							<td>
								{#if status === 'active'}
									<button class="btn btn-sm preset-filled-error-500" onclick={() => (confirmRevokeId = token.id)}>
										Revoke
									</button>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		{#if tokens.total_pages > 1}
			<p class="mt-2 text-sm text-surface-500">
				Page {tokens.page} of {tokens.total_pages} ({tokens.total} total)
			</p>
		{/if}
	{:else if tokens}
		<p class="text-surface-600 dark:text-surface-400">No enrollment tokens configured.</p>
	{/if}
</div>

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
