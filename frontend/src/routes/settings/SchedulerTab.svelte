<script lang="ts">
	import { onMount } from 'svelte';
	import { listSchedulerTasks, updateSchedulerTask, triggerSchedulerTask } from '$lib/api';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import { formatDate } from '$lib/utils';
	import { getUser } from '$lib/auth.svelte';
	import { Permission } from '$lib/types';
	import type { ScheduledTaskResponse } from '$lib/types';
	import { Callout, DataTable, FormFieldRow, ModalShell, SectionCard, StatusBadge } from '$lib/components/ui';
	import type { DataTableColumn } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageScheduler) ?? false);

	let tasks: ScheduledTaskResponse[] = $state([]);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);
	let editingTask: ScheduledTaskResponse | null = $state(null);
	let editInterval: number = $state(300);
	let editJitter: number = $state(0);
	let editEnabled: boolean = $state(true);
	let saving: boolean = $state(false);
	let triggeringId: string | null = $state(null);

	const columns: DataTableColumn[] = [
		{ key: 'task', label: 'Task' },
		{ key: 'schedule', label: 'Schedule' },
		{ key: 'status', label: 'Status' },
		{ key: 'last_run', label: 'Last Run' },
		{ key: 'next_run', label: 'Next Run' },
		{ key: 'actions', label: 'Actions', align: 'right' }
	];

	function formatInterval(seconds: number): string {
		if (seconds % 3600 === 0) return `${seconds / 3600}h`;
		if (seconds % 60 === 0) return `${seconds / 60}m`;
		return `${seconds}s`;
	}

	onMount(async () => {
		if (canManage) await loadTasks();
	});

	async function loadTasks() {
		loading = true;
		error = null;
		try {
			tasks = await listSchedulerTasks();
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load scheduler tasks';
		} finally {
			loading = false;
		}
	}

	function openEdit(task: ScheduledTaskResponse) {
		editingTask = task;
		editInterval = task.interval_seconds;
		editJitter = task.jitter_seconds;
		editEnabled = task.enabled;
	}

	function closeEdit() {
		editingTask = null;
	}

	async function saveEdit() {
		if (!editingTask || saving) return;
		saving = true;
		try {
			const updated = await updateSchedulerTask(editingTask.id, {
				interval_seconds: editInterval,
				jitter_seconds: editJitter,
				enabled: editEnabled
			});
			tasks = tasks.map((t) => (t.id === editingTask!.id ? updated : t));
			showSuccess('Task updated.');
			closeEdit();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to update task');
		} finally {
			saving = false;
		}
	}

	async function triggerNow(task: ScheduledTaskResponse) {
		triggeringId = task.id;
		try {
			const res = await triggerSchedulerTask(task.id);
			if (res.triggered) {
				showSuccess(`Task "${task.label}" triggered.`);
			} else {
				showSuccess(res.message);
			}
			await loadTasks();
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to trigger task');
		} finally {
			triggeringId = null;
		}
	}
</script>

{#if !canManage}
	<Callout tone="danger" title="Access denied" message="You do not have permission to manage the scheduler." />
{:else if loading}
	<SectionCard title="Scheduler">
		<p>Loading tasks...</p>
	</SectionCard>
{:else if error}
	<Callout tone="danger" title="Unable to load scheduler tasks" message={error}>
		<div class="mt-2">
			<Button variant="primary" onclick={loadTasks}>Retry</Button>
		</div>
	</Callout>
{:else}
	<SectionCard title="Scheduler">
		<DataTable
			{columns}
			rows={tasks as unknown as Record<string, unknown>[]}
			rowKey={(r) => (r as unknown as ScheduledTaskResponse).id}
			emptyTitle="No scheduled tasks configured"
		>
			{#snippet row(r)}
				{@const task = r as unknown as ScheduledTaskResponse}
				<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
					<td class="table-cell-pad">
						<p class="font-medium">{task.label}</p>
						<p class="text-table-header text-[var(--text-muted)]">{task.task_type}</p>
					</td>
					<td class="table-cell-pad">
						<code class="text-sm">{formatInterval(task.interval_seconds)}</code>
						{#if task.jitter_seconds > 0}
							<span class="text-table-header text-[var(--text-muted)]">±{formatInterval(task.jitter_seconds)}</span>
						{/if}
					</td>
					<td class="table-cell-pad">
						{#if task.is_running}
							<StatusBadge tone="warning" label="Running" />
						{:else if task.enabled}
							<StatusBadge tone="success" label="Enabled" />
						{:else}
							<StatusBadge tone="neutral" label="Disabled" />
						{/if}
						{#if task.last_error}
							<p class="mt-1 text-table-header text-[var(--color-danger)]" title={task.last_error}>Last error</p>
						{/if}
					</td>
					<td class="table-cell-pad">{formatDate(task.last_run_at)}</td>
					<td class="table-cell-pad">{formatDate(task.next_run_at)}</td>
					<td class="table-cell-pad text-right">
						<div class="flex justify-end gap-1">
							<Button variant="secondary" size="sm" onclick={() => openEdit(task)}>Edit</Button>
							<Button
								variant="ghost"
								size="sm"
								loading={triggeringId === task.id}
								disabled={task.is_running}
								onclick={() => triggerNow(task)}>Run</Button
							>
						</div>
					</td>
				</tr>
			{/snippet}
		</DataTable>
	</SectionCard>
{/if}

{#if editingTask}
	<ModalShell title="Edit Task: {editingTask.label}" onclose={closeEdit}>
		<FormFieldRow label="Interval (seconds)" inputId="scheduler-interval">
			<input id="scheduler-interval" class="input" type="number" min="1" bind:value={editInterval} />
		</FormFieldRow>
		<FormFieldRow label="Jitter (seconds)" inputId="scheduler-jitter">
			<input id="scheduler-jitter" class="input" type="number" min="0" bind:value={editJitter} />
		</FormFieldRow>
		<FormFieldRow label="Task State" inputId="scheduler-enabled">
			<label class="flex items-center gap-3">
				<input id="scheduler-enabled" class="checkbox" type="checkbox" bind:checked={editEnabled} />
				<span>Enabled</span>
			</label>
		</FormFieldRow>
		{#snippet footer()}
			<Button variant="secondary" onclick={closeEdit}>Cancel</Button>
			<Button variant="primary" loading={saving} onclick={saveEdit}>Save</Button>
		{/snippet}
	</ModalShell>
{/if}
