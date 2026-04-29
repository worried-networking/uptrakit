<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { getUser } from '$lib/auth.svelte';
	import {
		getHostTags,
		createHostTag,
		updateHostTag,
		deleteHostTag,
		batchHostTags,
		executeBatchChunked
	} from '$lib/api';
	import type { HostTagResponse, BatchActionResponse } from '$lib/types';
	import { Permission, hasAnyPermission } from '$lib/types';
	import { formatDate, parseUrlPage, nextValidPage } from '$lib/utils';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import { subscribeToEvent } from '$lib/stores/events.svelte';
	import Button from '$lib/components/Button.svelte';
	import { Checkbox, Input, Textarea } from '$lib/components/forms';
	import EllipsisIcon from '$lib/components/icons/EllipsisIcon.svelte';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import BatchActionBar from '$lib/components/BatchActionBar.svelte';
	import BatchResultDialog from '$lib/components/BatchResultDialog.svelte';
	import {
		ContextMenuItem,
		ContextMenuShell,
		DataTable,
		ModalShell,
		PageShell,
		SectionCard,
		StatusBadge,
		TableFooterBar
	} from '$lib/components/ui';
	import { FormFieldRow } from '$lib/components/forms';

	let tags: HostTagResponse[] = $state([]);
	let error: string | null = $state(null);
	let openMenuId: string | null = $state(null);
	let menuPos: { top: number; left: number } = $state({ top: 0, left: 0 });
	let confirmAction: { tagId: string; action: 'delete'; name: string } | null = $state(null);
	let submitting: boolean = $state(false);
	let currentPage: number = $state(parseUrlPage(page.url));
	let totalPages: number = $state(1);
	let totalItems: number = $state(0);
	let searchQuery: string = $state('');
	let searchTimeout: ReturnType<typeof setTimeout> | null = null;

	function getDefaultTagColor(): string {
		if (typeof document === 'undefined') return '#06b6d4';
		const val = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
		return val || '#06b6d4';
	}

	let showCreateModal: boolean = $state(false);
	let editTag: { id: string; name: string; color: string; description: string } | null = $state(null);
	let createForm = $state({ name: '', color: '', description: '' });

	let selectedIds = new SvelteSet<string>();
	let batchConfirmAction: string | null = $state(null);
	let batchResult: BatchActionResponse | null = $state(null);
	let selectingAllPages = $state(false);
	let isRetrying: boolean = $state(false);

	const allPageSelected = $derived(tags.length > 0 && tags.every((t) => selectedIds.has(t.id)));

	const selectAllPagesInfo = $derived(
		allPageSelected && totalItems > tags.length && selectedIds.size < totalItems
			? { total: totalItems, loading: selectingAllPages, onSelect: selectAllPages }
			: undefined
	);

	const batchActions: { id: string; label: string; destructive?: boolean }[] = [
		{ id: 'delete', label: 'Delete', destructive: true }
	];

	let refreshInterval: ReturnType<typeof setInterval> | null = null;
	let unsubscribers: (() => void)[] = [];

	$effect(() => {
		const search = currentPage > 1 ? `page=${currentPage}` : '';
		goto(search ? `${location.pathname}?${search}` : location.pathname, {
			replaceState: true,
			keepFocus: true,
			noScroll: true
		});
	});

	onMount(() => {
		loadTags(currentPage);
		unsubscribers.push(
			subscribeToEvent('host_tag_created', () => loadTags(currentPage, true)),
			subscribeToEvent('host_tag_updated', () => loadTags(currentPage, true)),
			subscribeToEvent('host_tag_deleted', () => loadTags(currentPage, true))
		);
		refreshInterval = setInterval(() => {
			if (document.visibilityState === 'visible') loadTags(currentPage, true);
		}, 300_000);
	});

	onDestroy(() => {
		for (const unsub of unsubscribers) unsub();
		if (refreshInterval) clearInterval(refreshInterval);
		if (searchTimeout) clearTimeout(searchTimeout);
	});

	async function loadTags(pg: number, background = false) {
		try {
			if (!background) error = null;
			const result = await getHostTags(pg, undefined, searchQuery || undefined);
			tags = result.items;
			currentPage = result.page;
			totalPages = result.total_pages;
			totalItems = result.total;
			if (background) error = null;
		} catch (e) {
			if (!background) {
				error = e instanceof Error ? e.message : 'Failed to load tags';
			}
		}
	}

	function handleSearchInput(event: Event) {
		const value = (event.target as HTMLInputElement).value;
		searchQuery = value;
		if (searchTimeout) clearTimeout(searchTimeout);
		searchTimeout = setTimeout(() => {
			selectedIds.clear();
			currentPage = 1;
			loadTags(1);
		}, 300);
	}

	function toggleMenu(id: string, button: HTMLElement) {
		if (openMenuId === id) {
			openMenuId = null;
			return;
		}
		const rect = button.getBoundingClientRect();
		menuPos = { top: rect.bottom + 4, left: rect.right - 160 };
		openMenuId = id;
	}

	function closeMenu() {
		openMenuId = null;
	}

	function requestConfirm(tagId: string, action: 'delete', name: string) {
		closeMenu();
		confirmAction = { tagId, action, name };
	}

	function cancelConfirm() {
		confirmAction = null;
	}

	function openCreateDialog() {
		createForm = { name: '', color: '', description: '' };
		showCreateModal = true;
	}

	function openEditDialog(tag: HostTagResponse) {
		closeMenu();
		editTag = { id: tag.id, name: tag.name, color: tag.color, description: tag.description ?? '' };
	}

	function cancelEdit() {
		editTag = null;
	}

	async function executeCreate() {
		if (submitting) return;
		submitting = true;
		try {
			error = null;
			await createHostTag({
				name: createForm.name,
				color: createForm.color || undefined,
				description: createForm.description || undefined
			});
			showCreateModal = false;
			showSuccess('Tag created');
			await loadTags(currentPage);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to create tag');
		} finally {
			submitting = false;
		}
	}

	async function executeEdit() {
		if (!editTag || submitting) return;
		submitting = true;
		try {
			error = null;
			const updated = await updateHostTag(editTag.id, {
				name: editTag.name,
				color: editTag.color,
				description: editTag.description || null
			});
			tags = tags.map((t) => (t.id === editTag?.id ? updated : t));
			editTag = null;
			showSuccess('Tag updated');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update tag');
		} finally {
			submitting = false;
		}
	}

	async function executeConfirmed() {
		if (!confirmAction || submitting) return;
		const { tagId } = confirmAction;
		confirmAction = null;
		submitting = true;
		try {
			error = null;
			await deleteHostTag(tagId);
			showSuccess('Tag deleted');
			await loadTags(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadTags(p);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to delete tag');
		} finally {
			submitting = false;
		}
	}

	function toggleSelectAll() {
		if (allPageSelected) {
			for (const t of tags) selectedIds.delete(t.id);
		} else {
			for (const t of tags) selectedIds.add(t.id);
		}
	}

	function toggleSelect(id: string) {
		if (selectedIds.has(id)) {
			selectedIds.delete(id);
		} else {
			selectedIds.add(id);
		}
	}

	function requestBatchAction(actionId: string) {
		batchConfirmAction = actionId;
	}

	async function selectAllPages() {
		selectingAllPages = true;
		try {
			let p = 1;
			while (true) {
				const result = await getHostTags(p, 100, searchQuery || undefined);
				for (const tag of result.items) selectedIds.add(tag.id);
				if (p >= result.total_pages) break;
				p++;
			}
		} catch {
			showError('Failed to select all items');
		} finally {
			selectingAllPages = false;
		}
	}

	async function executeBatchAction() {
		if (!batchConfirmAction || submitting) return;
		const action = batchConfirmAction;
		batchConfirmAction = null;
		submitting = true;
		try {
			const response = await executeBatchChunked(action, [...selectedIds], batchHostTags);
			if (response.failed.length > 0) {
				batchResult = response;
			} else {
				showSuccess(`${response.succeeded.length} tag(s) deleted successfully.`);
			}
			selectedIds.clear();
			await loadTags(currentPage);
			const p = nextValidPage(currentPage, totalPages);
			if (p !== null) await loadTags(p);
		} catch (e) {
			showError(e instanceof Error ? e.message : `Failed to ${action} tags`);
		} finally {
			submitting = false;
		}
	}

	function handleWindowClick(event: MouseEvent) {
		if (openMenuId && !(event.target as HTMLElement).closest('.actions-menu')) {
			closeMenu();
		}
	}

	const canManage = $derived(hasAnyPermission(getUser(), Permission.UpdateHosts, Permission.DeactivateHosts));
</script>

<svelte:window onclick={handleWindowClick} />

{#if getUser()}
	<PageShell title="Host Tags" description="Organize hosts into reusable groups for targeting and discovery.">
		{#snippet actions()}
			{#if canManage}
				<Button variant="primary" onclick={openCreateDialog}>Create Tag</Button>
			{/if}
		{/snippet}

		<SectionCard title="Search">
			<Input
				id="search-tags"
				type="text"
				placeholder="Search tags..."
				value={searchQuery}
				oninput={handleSearchInput}
			/>
		</SectionCard>

		<SectionCard title="Tags">
			<DataTable
				columns={[]}
				rows={tags as unknown as Record<string, unknown>[]}
				{error}
				emptyTitle="No tags yet"
				emptyDescription="Create a tag to organize and group your hosts."
				rowKey={(row) => (row as unknown as HostTagResponse).id}
			>
				{#snippet header()}
					<tr class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]">
						{#if canManage}
							<th
								class="w-10 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							>
								<Checkbox
									id="tag-batch-select-all"
									checked={allPageSelected}
									indeterminate={!allPageSelected && selectedIds.size > 0}
									onchange={toggleSelectAll}
									aria-label="Select all"
								/>
							</th>
						{/if}
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Name</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						>
							Description
						</th>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						>
							Host Count
						</th>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Created</th
						>
						{#if canManage}
							<th
								class="w-20 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header sticky right-0 bg-[var(--bg-raised)]"
								scope="col"
							></th>
						{/if}
					</tr>
				{/snippet}
				{#snippet row(rowValue, _index)}
					{@const tag = rowValue as unknown as HostTagResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						{#if canManage}
							<td class="table-cell-pad">
								<Checkbox
									id="tag-row-{tag.id}"
									checked={selectedIds.has(tag.id)}
									onchange={() => toggleSelect(tag.id)}
									aria-label="Select {tag.name}"
								/>
							</td>
						{/if}
						<td class="table-cell-pad text-[var(--text-primary)]">
							<span class="inline-flex items-center gap-2">
								<span class="inline-block h-3 w-3 rounded-full flex-shrink-0" style="background-color: {tag.color}"
								></span>
								<span class="font-medium">{tag.name}</span>
							</span>
						</td>
						<td class="table-cell-pad text-[var(--text-primary)]">{tag.description ?? '\u2014'}</td>
						<td class="table-cell-pad text-[var(--text-primary)]">
							<StatusBadge
								tone={tag.host_count > 0 ? 'info' : 'neutral'}
								label={tag.host_count === 1 ? '1 host' : `${tag.host_count} hosts`}
							/>
						</td>
						<td class="table-cell-pad text-[var(--text-primary)]">{formatDate(tag.created_at)}</td>
						{#if canManage}
							<td class="table-cell-pad sticky right-0 bg-[var(--bg-surface)]">
								<div class="actions-menu">
									<Button
										variant="ghost"
										size="sm"
										ariaLabel="Actions for {tag.name}"
										onclick={(e) => {
											e.stopPropagation();
											toggleMenu(tag.id, e.currentTarget);
										}}
									>
										{#snippet leadingIcon()}<EllipsisIcon />{/snippet}
										<span class="sr-only">Actions for {tag.name}</span>
									</Button>
								</div>
							</td>
						{/if}
					</tr>
				{/snippet}
				{#snippet errorActions()}
					<Button
						variant="primary"
						loading={isRetrying}
						onclick={async () => {
							isRetrying = true;
							try {
								await loadTags(currentPage);
							} finally {
								isRetrying = false;
							}
						}}
						class="mt-3"
					>
						Retry
					</Button>
				{/snippet}
				{#snippet footer()}
					{#if !error}
						<TableFooterBar {currentPage} {totalPages} total={totalItems} onPageChange={loadTags} />
					{/if}
				{/snippet}
			</DataTable>
		</SectionCard>
	</PageShell>

	{#if canManage && selectedIds.size > 0}
		<BatchActionBar
			selectedCount={selectedIds.size}
			actions={batchActions}
			onaction={requestBatchAction}
			oncancel={() => selectedIds.clear()}
			selectAllPages={selectAllPagesInfo}
		/>
	{/if}

	{#if batchConfirmAction}
		<ConfirmDialog
			title="Batch Delete"
			messagePrefix="Are you sure you want to delete"
			entityName="{selectedIds.size} tag(s)"
			confirmLabel={submitting ? 'Processing...' : 'Delete'}
			confirmDisabled={submitting}
			onconfirm={executeBatchAction}
			oncancel={() => (batchConfirmAction = null)}
		/>
	{/if}

	{#if batchResult}
		<BatchResultDialog title="Batch Action Results" response={batchResult} onclose={() => (batchResult = null)} />
	{/if}

	{#if openMenuId}
		{@const tag = tags.find((t) => t.id === openMenuId)}
		{#if tag}
			<ContextMenuShell top={menuPos.top} left={menuPos.left} onclose={closeMenu}>
				<li>
					<ContextMenuItem label="Edit" onclick={() => openEditDialog(tag)} />
				</li>
				<li>
					<ContextMenuItem label="Delete" destructive onclick={() => requestConfirm(tag.id, 'delete', tag.name)} />
				</li>
			</ContextMenuShell>
		{/if}
	{/if}

	{#if confirmAction}
		<ConfirmDialog
			title="Delete Tag"
			messagePrefix="Are you sure you want to delete"
			entityName={confirmAction.name}
			confirmLabel={submitting ? 'Processing...' : 'Delete'}
			confirmDisabled={submitting}
			onconfirm={executeConfirmed}
			oncancel={cancelConfirm}
		/>
	{/if}

	{#if showCreateModal}
		<ModalShell title="Create Tag" onclose={() => (showCreateModal = false)}>
			<div class="space-y-4">
				<FormFieldRow label="Name" inputId="create-tag-name">
					<Input id="create-tag-name" type="text" bind:value={createForm.name} placeholder="e.g. production" />
				</FormFieldRow>
				<FormFieldRow label="Color" hint="Optional" inputId="create-tag-color-text">
					<div class="flex items-center gap-2">
						{#if createForm.color}
							<input
								type="color"
								bind:value={createForm.color}
								class="h-10 w-10 cursor-pointer rounded-panel border-0 flex-shrink-0"
							/>
							<Input
								id="create-tag-color-text"
								type="text"
								bind:value={createForm.color}
								placeholder="#RRGGBB"
								class="flex-1"
							/>
							<Button variant="secondary" size="sm" class="flex-shrink-0" onclick={() => (createForm.color = '')}>
								Auto
							</Button>
						{:else}
							<span class="text-[var(--text-muted)] text-sm flex-1">Auto-assigned from palette</span>
							<Button
								variant="secondary"
								size="sm"
								class="flex-shrink-0"
								onclick={() => (createForm.color = getDefaultTagColor())}
							>
								Pick color
							</Button>
						{/if}
					</div>
				</FormFieldRow>
				<FormFieldRow label="Description" hint="Optional" inputId="create-tag-description">
					<Textarea
						id="create-tag-description"
						bind:value={createForm.description}
						rows={2}
						placeholder="Optional description"
					/>
				</FormFieldRow>
			</div>
			{#snippet footer()}
				<Button variant="secondary" onclick={() => (showCreateModal = false)}>Cancel</Button>
				<Button variant="primary" loading={submitting} disabled={!createForm.name.trim()} onclick={executeCreate}>
					Create
				</Button>
			{/snippet}
		</ModalShell>
	{/if}

	{#if editTag}
		<ModalShell title="Edit Tag" onclose={cancelEdit}>
			<div class="space-y-4">
				<FormFieldRow label="Name" inputId="edit-tag-name">
					<Input id="edit-tag-name" type="text" bind:value={editTag.name} />
				</FormFieldRow>
				<FormFieldRow label="Color" inputId="edit-tag-color-text">
					<div class="flex items-center gap-2">
						<input type="color" bind:value={editTag.color} class="h-10 w-10 cursor-pointer rounded-panel border-0" />
						<Input id="edit-tag-color-text" type="text" bind:value={editTag.color} class="flex-1" />
					</div>
				</FormFieldRow>
				<FormFieldRow label="Description" hint="Optional" inputId="edit-tag-description">
					<Textarea id="edit-tag-description" bind:value={editTag.description} rows={2} />
				</FormFieldRow>
			</div>
			{#snippet footer()}
				<Button variant="secondary" onclick={cancelEdit}>Cancel</Button>
				<Button variant="primary" loading={submitting} disabled={!editTag?.name.trim()} onclick={executeEdit}>
					Save
				</Button>
			{/snippet}
		</ModalShell>
	{/if}
{/if}
