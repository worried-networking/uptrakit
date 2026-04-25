<script lang="ts">
	import {
		listNotificationChannels,
		listNotificationRules,
		createNotificationRule,
		updateNotificationRule,
		deleteNotificationRule
	} from '$lib/api';
	import type { NotificationChannelSummary, NotificationRuleResponse, NotificationEventType } from '$lib/types';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import {
		DataTable,
		FormFieldRow,
		SectionCard,
		StatusBadge,
		TableFooterBar,
		type DataTableColumn
	} from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import Checkbox from '$lib/components/Checkbox.svelte';
	import Input from '$lib/components/Input.svelte';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	const EVENT_TYPE_LABELS: Record<NotificationEventType, string> = {
		update_available: 'Update Available',
		update_completed: 'Update Completed',
		update_failed: 'Update Failed',
		new_software_discovered: 'New Software Discovered',
		new_service_enrolled: 'New Service Enrolled',
		ca_rotated: 'CA Rotated',
		batch_update_completed: 'Batch Update Completed',
		batch_update_partially_completed: 'Batch Update Partially Completed',
		stdin_attention: 'Input Attention Required'
	};

	let rules: NotificationRuleResponse[] = $state([]);
	let channels: NotificationChannelSummary[] = $state([]);
	let channelMap: Map<string, string> = $derived(new Map(channels.map((c) => [c.id, c.name])));
	let loading: boolean = $state(true);
	let currentPage: number = $state(1);
	let totalPages: number = $state(1);
	let totalCount: number = $state(0);
	let showModal: boolean = $state(false);
	let editingRule: NotificationRuleResponse | null = $state(null);
	let saving: boolean = $state(false);
	let deleteConfirm: { id: string; eventType: string } | null = $state(null);

	let form = $state({
		channel_id: '',
		event_type: '' as string,
		host_id: '',
		software_item_id: '',
		plugin_type: '',
		enabled: true
	});

	$effect(() => {
		void loadData();
	});

	async function loadData() {
		loading = true;
		try {
			const [rulesRes, channelsRes] = await Promise.all([
				listNotificationRules({ page: currentPage }),
				listNotificationChannels(1, 1000)
			]);
			rules = rulesRes.items;
			totalPages = rulesRes.total_pages;
			totalCount = rulesRes.total;
			channels = channelsRes.items;
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to load notification rules');
		} finally {
			loading = false;
		}
	}

	function openCreate() {
		editingRule = null;
		form = {
			channel_id: channels.length > 0 ? channels[0].id : '',
			event_type: 'update_available',
			host_id: '',
			software_item_id: '',
			plugin_type: '',
			enabled: true
		};
		showModal = true;
	}

	function openEdit(rule: NotificationRuleResponse) {
		editingRule = rule;
		form = {
			channel_id: rule.channel_id,
			event_type: rule.event_type,
			host_id: rule.host_id ?? '',
			software_item_id: rule.software_item_id ?? '',
			plugin_type: rule.plugin_type ?? '',
			enabled: rule.enabled
		};
		showModal = true;
	}

	async function saveRule() {
		saving = true;
		try {
			const data: Record<string, unknown> = {
				channel_id: form.channel_id,
				event_type: form.event_type,
				enabled: form.enabled
			};
			if (form.host_id) data.host_id = form.host_id;
			if (form.software_item_id) data.software_item_id = form.software_item_id;
			if (form.plugin_type) data.plugin_type = form.plugin_type;

			if (editingRule) {
				await updateNotificationRule(editingRule.id, data);
				onSuccess('Rule updated');
			} else {
				await createNotificationRule(data as Parameters<typeof createNotificationRule>[0]);
				onSuccess('Rule created');
			}
			showModal = false;
			await loadData();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save rule');
		} finally {
			saving = false;
		}
	}

	async function confirmDelete() {
		if (!deleteConfirm) return;
		try {
			await deleteNotificationRule(deleteConfirm.id);
			onSuccess('Rule deleted');
			deleteConfirm = null;
			await loadData();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to delete rule');
		}
	}

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleString();
	}

	function scopeLabel(rule: NotificationRuleResponse): string {
		const parts: string[] = [];
		if (rule.host_id) parts.push(`Host: ${rule.host_id.slice(0, 8)}...`);
		if (rule.software_item_id) parts.push(`Item: ${rule.software_item_id.slice(0, 8)}...`);
		if (rule.plugin_type) parts.push(`Plugin: ${rule.plugin_type}`);
		return parts.length > 0 ? parts.join(', ') : 'All';
	}

	const rulesColumns: DataTableColumn[] = [
		{ key: 'channel', label: 'Channel' },
		{ key: 'event_type', label: 'Event Type' },
		{ key: 'scope', label: 'Scope' },
		{ key: 'enabled', label: 'Enabled' },
		{ key: 'created_at', label: 'Created' },
		{ key: 'actions', label: 'Actions', align: 'right' }
	];
</script>

<SectionCard title="Notification Rules">
	<div class="flex items-center justify-between mb-4">
		<Button variant="primary" size="sm" onclick={openCreate}>Add Rule</Button>
	</div>

	{#if loading}
		<p class="text-center text-[var(--text-muted)]">Loading rules...</p>
	{:else if rules.length === 0}
		<p class="text-center text-[var(--text-muted)]">No notification rules configured.</p>
	{:else}
		<DataTable
			columns={rulesColumns}
			rows={rules as unknown as Record<string, unknown>[]}
			loading={false}
			emptyTitle="No notification rules configured."
			rowKey={(row) => (row as unknown as NotificationRuleResponse).id}
		>
			{#snippet row(rowValue, _index)}
				{@const rule = rowValue as unknown as NotificationRuleResponse}
				<tr class="border-b border-[var(--border-subtle)] last:border-b-0 even:bg-[var(--bg-raised)]">
					<td class="table-cell-pad">{channelMap.get(rule.channel_id) ?? rule.channel_id.slice(0, 8)}</td>
					<td class="table-cell-pad"
						>{EVENT_TYPE_LABELS[rule.event_type as NotificationEventType] ?? rule.event_type}</td
					>
					<td class="table-cell-pad text-sm text-[var(--text-muted)]">{scopeLabel(rule)}</td>
					<td class="table-cell-pad">
						{#if rule.enabled}
							<StatusBadge tone="success" label="Yes" />
						{:else}
							<StatusBadge tone="neutral" label="No" />
						{/if}
					</td>
					<td class="table-cell-pad text-sm">{formatDate(rule.created_at)}</td>
					<td class="table-cell-pad text-right">
						<Button variant="secondary" size="sm" onclick={() => openEdit(rule)}>Edit</Button>
						<Button
							variant="danger"
							size="sm"
							onclick={() => (deleteConfirm = { id: rule.id, eventType: rule.event_type })}>Delete</Button
						>
					</td>
				</tr>
			{/snippet}
			{#snippet footer()}
				{#if totalPages > 1}
					<TableFooterBar
						total={totalCount}
						{currentPage}
						{totalPages}
						onPageChange={(page) => {
							currentPage = page;
							void loadData();
						}}
					/>
				{/if}
			{/snippet}
		</DataTable>
	{/if}
</SectionCard>

{#if showModal}
	<Modal title={editingRule ? 'Edit Rule' : 'Add Rule'} maxWidth="max-w-lg" onclose={() => (showModal = false)}>
		<form
			onsubmit={(e) => {
				e.preventDefault();
				void saveRule();
			}}
			class="space-y-4"
		>
			<FormFieldRow label="Channel" inputId="rule-channel-id" required>
				<select id="rule-channel-id" bind:value={form.channel_id} required class="select">
					{#each channels as ch (ch.id)}
						<option value={ch.id}>{ch.name} ({ch.channel_type})</option>
					{/each}
				</select>
			</FormFieldRow>

			<FormFieldRow label="Event Type" inputId="rule-event-type" required>
				<select id="rule-event-type" bind:value={form.event_type} required class="select">
					{#each Object.entries(EVENT_TYPE_LABELS) as [value, label] (value)}
						<option {value}>{label}</option>
					{/each}
				</select>
			</FormFieldRow>

			<FormFieldRow label="Host ID" inputId="rule-host-id">
				<Input
					id="rule-host-id"
					type="text"
					bind:value={form.host_id}
					placeholder="Optional — scope to a specific host"
				/>
			</FormFieldRow>

			<FormFieldRow label="Software Item ID" inputId="rule-software-item-id">
				<Input
					id="rule-software-item-id"
					type="text"
					bind:value={form.software_item_id}
					placeholder="Optional — scope to a specific software item"
				/>
			</FormFieldRow>

			<FormFieldRow label="Plugin Type" inputId="rule-plugin-type">
				<Input
					id="rule-plugin-type"
					type="text"
					bind:value={form.plugin_type}
					placeholder="Optional — scope to a plugin type"
				/>
			</FormFieldRow>

			<label class="flex items-center gap-2">
				<Checkbox id="notification-rule-enabled" bind:checked={form.enabled} />
				<span class="text-sm">Enabled</span>
			</label>

			<Button type="submit" variant="primary" loading={saving}>
				{editingRule ? 'Update' : 'Create'}
			</Button>
		</form>
	</Modal>
{/if}

{#if deleteConfirm}
	<ConfirmDialog
		title="Delete Rule"
		messagePrefix="Are you sure you want to delete the rule for"
		entityName={EVENT_TYPE_LABELS[deleteConfirm.eventType as NotificationEventType] ?? deleteConfirm.eventType}
		confirmLabel="Delete"
		onconfirm={() => void confirmDelete()}
		oncancel={() => (deleteConfirm = null)}
	/>
{/if}
