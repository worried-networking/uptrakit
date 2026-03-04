<script lang="ts">
	import { listSystemEnrollmentTokens, createSystemEnrollmentToken, revokeSystemEnrollmentToken } from '$lib/api';
	import type {
		SystemEnrollmentTokenResponse,
		SystemEnrollmentTokenCreatedResponse,
		PaginatedResponse,
		CreateSystemEnrollmentTokenRequest
	} from '$lib/types';
	import { copyToClipboard, formatDate } from '$lib/utils';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let tokens: PaginatedResponse<SystemEnrollmentTokenResponse> | null = $state(null);
	let loading: boolean = $state(false);
	let showCreateDialog: boolean = $state(false);
	let createdToken: SystemEnrollmentTokenCreatedResponse | null = $state(null);
	let copied: boolean = $state(false);

	// Create form fields
	let newName: string = $state('');
	let newMaxUses: string = $state('');
	let newExpiresIn: string = $state('');
	let creating: boolean = $state(false);

	async function loadTokens() {
		loading = true;
		try {
			tokens = await listSystemEnrollmentTokens();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to load system enrollment tokens');
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
			const data: CreateSystemEnrollmentTokenRequest = {
				name: newName.trim()
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
			setTimeout(() => {
				copied = false;
			}, 2000);
		}
	}

	function resetForm() {
		newName = '';
		newMaxUses = '';
		newExpiresIn = '';
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
		<h2 class="h3">System Services</h2>
		<div class="flex gap-2">
			{#if tokens === null}
				<button class="btn preset-filled-primary-500" onclick={loadTokens} disabled={loading}>
					{loading ? 'Loading...' : 'Load Tokens'}
				</button>
			{:else}
				<button class="btn preset-tonal" onclick={loadTokens} disabled={loading}>Refresh</button>
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

	<p class="mb-4 text-sm text-surface-600 dark:text-surface-400">
		Manage system enrollment tokens for infrastructure services such as the external scheduler and MQTT bridge. Services
		that present a valid token are automatically approved on enrollment. Tokens are shown only once at creation — store
		them securely.
	</p>

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
			<h3 class="h4 mb-3">Create System Enrollment Token</h3>
			<div class="space-y-3">
				<label class="label">
					<span>Name *</span>
					<input class="input" type="text" bind:value={newName} placeholder="e.g. MQTT Bridge Token" />
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
							<td>{formatUsage(token.current_uses, token.max_uses)}</td>
							<td>{token.expires_at ? formatDate(token.expires_at) : 'never'}</td>
							<td><span class="badge {statusBadgeClass(status)}">{status}</span></td>
							<td>{formatDate(token.created_at)}</td>
							<td>
								{#if status === 'active'}
									<button class="btn btn-sm preset-filled-error-500" onclick={() => handleRevoke(token.id)}>
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
		<p class="text-surface-600 dark:text-surface-400">No system enrollment tokens configured.</p>
	{/if}
</div>
