<script lang="ts">
	import { onMount } from 'svelte';
	import { listSchedulerTasks, updateSchedulerTask, triggerSchedulerTask } from '$lib/api';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import Modal from '$lib/components/Modal.svelte';
	import { formatDate } from '$lib/utils';
	import { getUser } from '$lib/auth.svelte';
	import { Permission } from '$lib/types';
	import type { ScheduledTaskResponse } from '$lib/types';

	const canManage = $derived(getUser()?.permissions.includes(Permission.ManageSoftware) ?? false);

	let tasks: ScheduledTaskResponse[] = $state([]);
	let loading: boolean = $state(true);
	let error: string | null = $state(null);
	let editingTask: ScheduledTaskResponse | null = $state(null);
	let editInterval: number = $state(300);
	let editJitter: number = $state(0);
	let editEnabled: boolean = $state(true);
	let saving: boolean = $state(false);
	let triggeringId: string | null = $state(null);

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
	<aside class="rounded-lg p-4 preset-filled-error-500">
		<p>You do not have permission to manage the scheduler.</p>
	</aside>
{:else if loading}
	<div class="card p-8 text-center">
		<p>Loading tasks...</p>
	</div>
{:else if error}
	<aside class="rounded-lg p-4 preset-filled-error-500">
		<p>{error}</p>
		<button class="btn preset-filled-primary-500 mt-2" onclick={loadTasks}>Retry</button>
	</aside>
{:else}
	<div class="table-wrap">
		<table class="table">
			<thead>
				<tr>
					<th>Task</th>
					<th>Schedule</th>
					<th>Status</th>
					<th>Last Run</th>
					<th>Next Run</th>
					<th class="w-40">Actions</th>
				</tr>
			</thead>
			<tbody>
				{#each tasks as task (task.id)}
					<tr>
						<td>
							<p class="font-medium">{task.label}</p>
							<p class="text-xs text-surface-500">{task.task_type}</p>
						</td>
						<td>
							<code class="text-sm">{formatInterval(task.interval_seconds)}</code>
							{#if task.jitter_seconds > 0}
								<span class="text-xs text-surface-500">±{formatInterval(task.jitter_seconds)}</span>
							{/if}
						</td>
						<td>
							{#if task.is_running}
								<span class="badge preset-filled-warning-500">Running</span>
							{:else if task.enabled}
								<span class="badge preset-filled-success-500">Enabled</span>
							{:else}
								<span class="badge preset-tonal">Disabled</span>
							{/if}
							{#if task.last_error}
								<p class="mt-1 text-xs text-error-500" title={task.last_error}>Last error</p>
							{/if}
						</td>
						<td>{formatDate(task.last_run_at)}</td>
						<td>{formatDate(task.next_run_at)}</td>
						<td>
							<div class="flex gap-1">
								<button class="btn btn-sm preset-tonal" onclick={() => openEdit(task)}>Edit</button>
								<button
									class="btn btn-sm preset-tonal"
									disabled={task.is_running || triggeringId === task.id}
									onclick={() => triggerNow(task)}
								>
									{triggeringId === task.id ? '...' : 'Run'}
								</button>
							</div>
						</td>
					</tr>
				{:else}
					<tr>
						<td colspan="6" class="py-8 text-center">No scheduled tasks configured.</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

{#if editingTask}
	<Modal title="Edit Task: {editingTask.label}" onclose={closeEdit}>
		<label class="label">
			<span>Interval (seconds)</span>
			<input class="input" type="number" min="1" bind:value={editInterval} />
		</label>
		<label class="label">
			<span>Jitter (seconds)</span>
			<input class="input" type="number" min="0" bind:value={editJitter} />
		</label>
		<label class="flex items-center gap-3">
			<input class="checkbox" type="checkbox" bind:checked={editEnabled} />
			<span>Enabled</span>
		</label>
		{#snippet footer()}
			<button class="btn preset-tonal-surface" onclick={closeEdit}>Cancel</button>
			<button class="btn preset-filled-primary-500" onclick={saveEdit} disabled={saving}>
				{saving ? 'Saving...' : 'Save'}
			</button>
		{/snippet}
	</Modal>
{/if}
