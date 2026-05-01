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
		EmptyState,
		ModalShell,
		PageShell,
		SectionCard,
		StatusBadge,
		TabStrip,
		type TabStripItem
	} from '$lib/components/ui';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { FormFieldRow, Input } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';

	const user = $derived(getUser());
	const authMethod = $derived(getAuthMethod());

	const tabItems: TabStripItem[] = [
		{ id: 'account', label: 'Account' },
		{ id: 'api-tokens', label: 'API Tokens' }
	];

	let activeTab = $state(page.url.searchParams.get('tab') ?? 'account');

	$effect(() => {
		const search = activeTab !== 'account' ? `?tab=${activeTab}` : '';
		goto(search ? `${location.pathname}${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

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
	let showChangeEmailModal = $state(false);
	let newEmail = $state('');
	let emailCurrentPassword = $state('');
	let emailChanging = $state(false);
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
			newEmail = '';
			emailCurrentPassword = '';
		} catch (e) {
			emailError = e instanceof Error ? e.message : 'Failed to initiate email change';
			return;
		} finally {
			emailChanging = false;
		}
		// Best-effort refresh — if this fails the modal stays open and the user
		// can close manually; don't overwrite emailError with a stale-data message.
		await initialize().catch(() => {});
	}

	async function handleCancelEmailChange() {
		if (!user) return;
		try {
			await cancelEmailChange(user.id);
			showSuccess('Email change cancelled');
			await initialize();
			showChangeEmailModal = false;
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
	let showChangePasswordModal = $state(false);
	let passwordError = $state('');

	function closePasswordModal() {
		showChangePasswordModal = false;
		currentPassword = '';
		newPassword = '';
		confirmPassword = '';
		confirmPasswordError = '';
		passwordError = '';
	}

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
			closePasswordModal();
			showSuccess('Password changed. Other sessions have been signed out.');
		} catch (e) {
			passwordError = e instanceof Error ? e.message : 'Failed to change password';
		} finally {
			passwordSaving = false;
		}
	}

	let tokens: ApiTokenResponse[] = $state([]);
	const activeTokens = $derived(tokens.filter((t) => t.revoked_at === null));
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
	<PageShell title="Profile">
		<TabStrip
			items={tabItems}
			activeId={activeTab}
			ariaLabel="Profile tabs"
			onSelect={(id) => {
				activeTab = id;
			}}
		/>

		{#if activeTab === 'account'}
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
							<Button variant="secondary" size="sm" onclick={() => (showChangeEmailModal = true)}>Change email</Button>
							{#if user.has_pending_email_change}
								<StatusBadge tone="warning" label="Change pending" />
							{/if}
						{/if}
					</FormFieldRow>
					{#if profileError}
						<Callout tone="danger" message={profileError} />
					{/if}
					<div class="flex justify-end">
						<Button variant="primary" loading={profileSaving} onclick={handleSaveProfile}>Save</Button>
					</div>
				</div>
			</SectionCard>

			<SectionCard title="Security">
				{#if authMethod === 'password'}
					<FormFieldRow label="Password">
						<span class="text-sm text-[var(--text-secondary)]">••••••••</span>
						<Button variant="secondary" size="sm" onclick={() => (showChangePasswordModal = true)}>Change</Button>
					</FormFieldRow>
				{:else}
					<Callout
						tone="info"
						message="Your account uses single sign-on. Password and email are managed by your identity provider."
					/>
				{/if}
			</SectionCard>
		{/if}

		{#if showChangeEmailModal}
			<ModalShell
				title="Change Email"
				onclose={() => {
					showChangeEmailModal = false;
				}}
				maxWidth="max-w-lg"
			>
				{#if user?.has_pending_email_change}
					<Callout
						tone="info"
						message="A confirmation email has been sent. Check your inbox. If you did not request this change, you can cancel it."
					/>
				{:else}
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
						<Callout tone="danger" message={emailError} />
					{/if}
				{/if}
				{#snippet footer()}
					{#if user?.has_pending_email_change}
						<Button variant="ghost" onclick={handleCancelEmailChange}>Cancel email change</Button>
						<Button variant="primary" onclick={() => (showChangeEmailModal = false)}>Close</Button>
					{:else}
						<Button variant="ghost" onclick={() => (showChangeEmailModal = false)}>Cancel</Button>
						<Button variant="primary" loading={emailChanging} onclick={handleInitiateEmailChange}>
							Send confirmation email
						</Button>
					{/if}
				{/snippet}
			</ModalShell>
		{/if}

		{#if showChangePasswordModal}
			<ModalShell title="Change Password" onclose={closePasswordModal} maxWidth="max-w-lg">
				<FormFieldRow label="Current password" inputId="pw-current">
					<Input id="pw-current" type="password" bind:value={currentPassword} placeholder="Current password" />
				</FormFieldRow>
				<FormFieldRow label="New password" inputId="pw-new" hint="8–128 characters.">
					<Input id="pw-new" type="password" bind:value={newPassword} placeholder="At least 8 characters" />
				</FormFieldRow>
				<FormFieldRow label="Confirm new password" inputId="pw-confirm" error={confirmPasswordError}>
					<Input id="pw-confirm" type="password" bind:value={confirmPassword} placeholder="Repeat new password" />
				</FormFieldRow>
				{#if passwordError}
					<Callout tone="danger" message={passwordError} />
				{/if}
				{#snippet footer()}
					<Button variant="ghost" onclick={closePasswordModal}>Cancel</Button>
					<Button variant="primary" loading={passwordSaving} onclick={handleChangePassword}>Change password</Button>
				{/snippet}
			</ModalShell>
		{/if}

		{#if activeTab === 'api-tokens'}
			<SectionCard
				title="API Tokens"
				description="API tokens allow programmatic access to Uptrakit. Treat tokens like passwords and rotate them regularly."
			>
				{#snippet actions()}
					<Button variant="primary" onclick={openCreateModal}>New Token</Button>
				{/snippet}

				{#if activeTokens.length === 0}
					<EmptyState title="No API tokens" description="Create a token to access Uptrakit programmatically.">
						{#snippet actions()}
							<Button variant="ghost" onclick={openCreateModal}>New Token</Button>
						{/snippet}
					</EmptyState>
				{:else}
					<DataTable
						columns={[]}
						rows={activeTokens as unknown as Record<string, unknown>[]}
						{loading}
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
									class="w-24 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
									scope="col"
								></th>
							</tr>
						{/snippet}
						{#snippet row(rowValue, _index)}
							{@const token = rowValue as unknown as ApiTokenResponse}
							<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
								<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{token.name}</td>
								<td class="table-cell-pad text-table-body text-[var(--text-primary)]">{formatDate(token.created_at)}</td
								>
								<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
									<Button
										variant="danger"
										size="sm"
										onclick={() => (revokeConfirm = { id: token.id, name: token.name })}
									>
										Revoke
									</Button>
								</td>
							</tr>
						{/snippet}
					</DataTable>
				{/if}
			</SectionCard>
		{/if}
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
