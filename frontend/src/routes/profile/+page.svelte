<script lang="ts">
	import { onMount } from 'svelte';
	import { getUser, getAuthMethod, initialize } from '$lib/auth.svelte';
	import {
		listApiTokens,
		createApiToken,
		revokeApiToken,
		updateProfile,
		initiateEmailChange,
		cancelEmailChange,
		changePassword
	} from '$lib/api';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import { formatDate } from '$lib/utils';
	import type { ApiTokenResponse } from '$lib/types';
	import {
		Callout,
		DataTable,
		FormFieldRow,
		ModalShell,
		PageShell,
		SectionCard,
		StatusBadge
	} from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';

	const user = $derived(getUser());
	const authMethod = $derived(getAuthMethod());
	// Profile details form
	let firstName = $state('');
	let lastName = $state('');
	let profileSaving = $state(false);
	let profileError = $state('');
	$effect(() => {
		if (user) {
			firstName = user.first_name;
			lastName = user.last_name;
		}
	});

	async function handleSaveProfile() {
		if (!user) return;
		profileSaving = true;
		profileError = '';
		try {
			await updateProfile(user.id, { first_name: firstName, last_name: lastName });
			showSuccess('Profile updated');
		} catch (e) {
			profileError = e instanceof Error ? e.message : 'Failed to update profile';
		} finally {
			profileSaving = false;
		}
	}

	// Change email form
	let showChangeEmail = $state(false);
	let newEmail = $state('');
	let emailCurrentPassword = $state('');
	let emailChanging = $state(false);
	let emailChangeSuccess = $state(false);
	let emailError = $state('');

	async function handleInitiateEmailChange() {
		if (!user) return;
		emailChanging = true;
		emailError = '';
		try {
			await initiateEmailChange(user.id, {
				new_email: newEmail,
				current_password: emailCurrentPassword
			});
			emailChangeSuccess = true;
			newEmail = '';
			emailCurrentPassword = '';
		} catch (e) {
			emailError = e instanceof Error ? e.message : 'Failed to initiate email change';
		} finally {
			emailChanging = false;
		}
	}

	async function handleCancelEmailChange() {
		if (!user) return;
		try {
			await cancelEmailChange(user.id);
			showSuccess('Email change cancelled');
			await initialize();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to cancel email change');
		}
	}

	// Change password form
	let currentPassword = $state('');
	let newPassword = $state('');
	let confirmPassword = $state('');
	let passwordSaving = $state(false);
	let confirmPasswordError = $state('');
	let passwordChangeSuccess = $state(false);
	let passwordError = $state('');

	async function handleChangePassword() {
		if (!user) return;
		if (newPassword !== confirmPassword) {
			confirmPasswordError = 'Passwords do not match';
			return;
		}
		confirmPasswordError = '';
		passwordError = '';
		passwordSaving = true;
		try {
			await changePassword(user.id, {
				current_password: currentPassword,
				new_password: newPassword
			});
			passwordChangeSuccess = true;
			currentPassword = '';
			newPassword = '';
			confirmPassword = '';
		} catch (e) {
			passwordError = e instanceof Error ? e.message : 'Failed to change password';
		} finally {
			passwordSaving = false;
		}
	}

	let tokens: ApiTokenResponse[] = $state([]);
	let loading: boolean = $state(true);
	let showCreateModal: boolean = $state(false);
	let newTokenName: string = $state('');
	let creating: boolean = $state(false);
	let createdToken: string | null = $state(null);
	let revokeConfirm: { id: string; name: string } | null = $state(null);
	let revoking: boolean = $state(false);

	onMount(async () => {
		await loadTokens();
	});

	async function loadTokens() {
		loading = true;
		try {
			const res = await listApiTokens();
			tokens = res.tokens;
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load API tokens');
		} finally {
			loading = false;
		}
	}

	function openCreateModal() {
		newTokenName = '';
		createdToken = null;
		showCreateModal = true;
	}

	function closeCreateModal() {
		showCreateModal = false;
		createdToken = null;
		newTokenName = '';
	}

	async function handleCreate() {
		if (!newTokenName.trim() || creating) return;
		creating = true;
		try {
			const res = await createApiToken({ name: newTokenName.trim() });
			tokens = [
				...tokens,
				{ id: res.id, name: newTokenName.trim(), revoked_at: null, created_at: new Date().toISOString() }
			];
			createdToken = res.token;
			newTokenName = '';
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to create API token');
			closeCreateModal();
		} finally {
			creating = false;
		}
	}

	async function handleRevoke() {
		if (!revokeConfirm || revoking) return;
		const { id } = revokeConfirm;
		revokeConfirm = null;
		revoking = true;
		try {
			await revokeApiToken(id);
			tokens = tokens.filter((t) => t.id !== id);
			showSuccess('API token revoked.');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to revoke API token');
		} finally {
			revoking = false;
		}
	}

	async function copyToken(token: string) {
		try {
			await navigator.clipboard.writeText(token);
			showSuccess('Token copied to clipboard.');
		} catch {
			showError('Failed to copy token. Please copy it manually.');
		}
	}
</script>

{#if user}
	<PageShell title="Profile" description="Manage your account information and API access tokens.">
		<SectionCard title="Profile">
			<div data-ui="profile-details-section">
				<FormFieldRow label="First name" inputId="profile-first-name">
					<Input id="profile-first-name" type="text" bind:value={firstName} placeholder="First name" />
				</FormFieldRow>
				<FormFieldRow label="Last name" inputId="profile-last-name">
					<Input id="profile-last-name" type="text" bind:value={lastName} placeholder="Last name" />
				</FormFieldRow>
				<FormFieldRow label="Email" inputId="profile-email">
					<Input id="profile-email" type="email" value={user?.email ?? ''} disabled />
					{#if authMethod === 'password'}
						<Button variant="secondary" size="sm" onclick={() => (showChangeEmail = true)}>Change email</Button>
					{/if}
				</FormFieldRow>
				{#if profileError}
					<Callout tone="danger">{profileError}</Callout>
				{/if}
				<div class="flex justify-end">
					<Button variant="primary" loading={profileSaving} onclick={handleSaveProfile}>Save</Button>
				</div>
			</div>
		</SectionCard>

		{#if authMethod === 'password'}
			<SectionCard title="Change email">
				<div data-ui="change-email-section">
					{#if emailChangeSuccess}
						<Callout tone="success">
							A confirmation link has been sent to your new address. Check your inbox and click the link to complete the
							change.
						</Callout>
					{:else if user?.has_pending_email_change}
						<Callout tone="info">
							A confirmation email has been sent. Check your inbox. If you did not request this change, you can cancel
							it.
						</Callout>
						<div class="flex justify-end">
							<Button variant="ghost" onclick={handleCancelEmailChange}>Cancel email change</Button>
						</div>
					{:else if showChangeEmail}
						<FormFieldRow label="New email address" inputId="email-new-email">
							<Input id="email-new-email" type="email" bind:value={newEmail} placeholder="new@example.com" />
						</FormFieldRow>
						<FormFieldRow label="Current password" inputId="email-current-password">
							<Input
								id="email-current-password"
								type="password"
								bind:value={emailCurrentPassword}
								placeholder="Enter your password"
							/>
						</FormFieldRow>
						{#if emailError}
							<Callout tone="danger">{emailError}</Callout>
						{/if}
						<div class="flex justify-end gap-2">
							<Button variant="ghost" onclick={() => (showChangeEmail = false)}>Cancel</Button>
							<Button variant="primary" loading={emailChanging} onclick={handleInitiateEmailChange}>
								Send confirmation email
							</Button>
						</div>
					{:else}
						<p class="text-sm text-[var(--text-secondary)]">
							Update your email address. A confirmation link will be sent to your new address.
						</p>
					{/if}
				</div>
			</SectionCard>
		{/if}

		{#if authMethod === 'password'}
			<SectionCard title="Change password">
				<div data-ui="change-password-section">
					{#if passwordChangeSuccess}
						<Callout tone="success">Password changed. Other sessions have been signed out.</Callout>
						<Button variant="secondary" onclick={() => (passwordChangeSuccess = false)}>Change again</Button>
					{:else}
						<FormFieldRow label="Current password" inputId="pw-current">
							<Input id="pw-current" type="password" bind:value={currentPassword} placeholder="Current password" />
						</FormFieldRow>
						<FormFieldRow label="New password" inputId="pw-new" hint="8–128 characters.">
							<Input id="pw-new" type="password" bind:value={newPassword} placeholder="At least 8 characters" />
						</FormFieldRow>
						<FormFieldRow label="Confirm new password" inputId="pw-confirm">
							<Input id="pw-confirm" type="password" bind:value={confirmPassword} placeholder="Repeat new password" />
							{#if confirmPasswordError}
								<p class="text-sm text-(--color-danger)">{confirmPasswordError}</p>
							{/if}
						</FormFieldRow>
						{#if passwordError}
							<Callout tone="danger">{passwordError}</Callout>
						{/if}
						<div class="flex justify-end">
							<Button variant="primary" loading={passwordSaving} onclick={handleChangePassword}>Change password</Button>
						</div>
					{/if}
				</div>
			</SectionCard>
		{/if}

		<SectionCard
			title="API Tokens"
			description="API tokens allow programmatic access to Uptrakit. Treat tokens like passwords and rotate them regularly."
		>
			{#snippet actions()}
				<Button variant="primary" onclick={openCreateModal}>New Token</Button>
			{/snippet}

			<DataTable
				columns={[]}
				rows={tokens as unknown as Record<string, unknown>[]}
				{loading}
				emptyTitle="No API tokens yet."
				rowKey={(row) => (row as unknown as ApiTokenResponse).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Name</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Created</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Status</th
						>
						<th
							class="w-24 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						></th>
					</tr>
				{/snippet}
				{#snippet row(rowValue)}
					{@const token = rowValue as unknown as ApiTokenResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{token.name}</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{formatDate(token.created_at)}</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
							{#if token.revoked_at}
								<StatusBadge tone="neutral" label="Revoked" />
							{:else}
								<StatusBadge tone="success" label="Active" />
							{/if}
						</td>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
							{#if !token.revoked_at}
								<Button variant="danger" size="sm" onclick={() => (revokeConfirm = { id: token.id, name: token.name })}>
									Revoke
								</Button>
							{/if}
						</td>
					</tr>
				{/snippet}
			</DataTable>
		</SectionCard>
	</PageShell>
{/if}

{#if revokeConfirm}
	<ConfirmDialog
		title="Revoke API Token"
		messagePrefix="Are you sure you want to revoke"
		entityName={revokeConfirm.name}
		confirmLabel={revoking ? 'Revoking...' : 'Revoke'}
		confirmDisabled={revoking}
		onconfirm={handleRevoke}
		oncancel={() => (revokeConfirm = null)}
	/>
{/if}

{#if showCreateModal}
	<ModalShell title="New API Token" onclose={closeCreateModal} maxWidth="max-w-lg">
		{#if createdToken}
			<Callout
				tone="warning"
				title="Save this token now"
				message="It will not be shown again after you close this dialog."
			/>
			<div class="relative">
				<pre
					class="rounded-panel bg-[var(--bg-raised)] p-3 font-mono text-sm break-all whitespace-pre-wrap">{createdToken}</pre>
			</div>
		{:else}
			<FormFieldRow label="Token Name" inputId="new-token-name">
				<Input
					id="new-token-name"
					type="text"
					placeholder="e.g. CI Pipeline"
					bind:value={newTokenName}
					onkeydown={(e) => {
						if (e.key === 'Enter') handleCreate();
					}}
				/>
			</FormFieldRow>
		{/if}
		{#snippet footer()}
			<div class="contents" data-ui="profile-token-modal-footer">
				{#if createdToken}
					<Button variant="secondary" onclick={() => copyToken(createdToken!)}>Copy</Button>
					<Button variant="primary" onclick={closeCreateModal}>Done</Button>
				{:else}
					<Button variant="secondary" onclick={closeCreateModal}>Cancel</Button>
					<Button variant="primary" onclick={handleCreate} disabled={!newTokenName.trim()} loading={creating}>
						Create
					</Button>
				{/if}
			</div>
		{/snippet}
	</ModalShell>
{/if}
