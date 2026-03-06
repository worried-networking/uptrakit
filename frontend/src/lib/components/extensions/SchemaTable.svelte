<script lang="ts">
	import type { ExtensionUi } from '$lib/types';
	import { invokeExtensionAction } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';
	import ActionButton from './ActionButton.svelte';

	let {
		extensionId,
		ui,
		serviceId
	}: {
		extensionId: string;
		ui: Extract<ExtensionUi, { type: 'data_table' }>;
		serviceId?: string;
	} = $props();

	let rows: Record<string, unknown>[] = $state([]);
	let loading: boolean = $state(true);

	async function loadData() {
		loading = true;
		try {
			const result = await invokeExtensionAction(extensionId, ui.data_action, {}, serviceId);
			rows = Array.isArray(result)
				? result
				: (((result as Record<string, unknown>)?.rows as Record<string, unknown>[]) ?? []);
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load data');
			rows = [];
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void loadData();
	});
</script>

<div class="card p-4">
	{#if ui.primary_actions.length > 0}
		<div class="mb-4 flex flex-wrap gap-2">
			{#each ui.primary_actions as action (action.action_id)}
				<ActionButton {extensionId} {action} {serviceId} onComplete={loadData} />
			{/each}
		</div>
	{/if}

	{#if loading}
		<p class="text-center py-8 text-surface-500">Loading...</p>
	{:else if rows.length === 0}
		<p class="text-center py-8 text-surface-500">No data available.</p>
	{:else}
		<div class="overflow-x-auto">
			<table class="w-full text-left text-sm">
				<thead>
					<tr class="border-b border-surface-300 dark:border-surface-600">
						{#each ui.columns as col (col.key)}
							<th class="px-3 py-2 font-medium">{col.label}</th>
						{/each}
						{#if ui.row_actions.length > 0}
							<th class="px-3 py-2 font-medium">Actions</th>
						{/if}
					</tr>
				</thead>
				<tbody>
					{#each rows as row, i (i)}
						<tr class="border-b border-surface-200 dark:border-surface-700">
							{#each ui.columns as col (col.key)}
								<td class="px-3 py-2">{String(row[col.key] ?? '')}</td>
							{/each}
							{#if ui.row_actions.length > 0}
								<td class="px-3 py-2">
									<div class="flex gap-1">
										{#each ui.row_actions as action (action.action_id)}
											<ActionButton
												{extensionId}
												{action}
												{serviceId}
												extraParams={{ _row: row }}
												size="sm"
												onComplete={loadData}
											/>
										{/each}
									</div>
								</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</div>
