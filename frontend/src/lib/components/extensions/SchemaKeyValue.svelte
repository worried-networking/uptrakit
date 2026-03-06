<script lang="ts">
	import { invokeExtensionAction } from '$lib/api';
	import { showError } from '$lib/notifications.svelte';

	let {
		extensionId,
		dataAction,
		serviceId
	}: {
		extensionId: string;
		dataAction: string;
		serviceId?: string;
	} = $props();

	let data: Record<string, unknown> = $state({});
	let loading: boolean = $state(true);

	async function loadData() {
		loading = true;
		try {
			const result = await invokeExtensionAction(extensionId, dataAction, {}, serviceId);
			data = (result as Record<string, unknown>) ?? {};
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to load data');
			data = {};
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void loadData();
	});
</script>

<div class="card p-4">
	{#if loading}
		<p class="text-center py-4 text-surface-500">Loading...</p>
	{:else if Object.keys(data).length === 0}
		<p class="text-center py-4 text-surface-500">No data available.</p>
	{:else}
		<dl class="divide-y divide-surface-200 dark:divide-surface-700">
			{#each Object.entries(data) as [key, value] (key)}
				<div class="flex justify-between py-2">
					<dt class="text-sm font-medium text-surface-600 dark:text-surface-400">{key}</dt>
					<dd class="text-sm">{String(value ?? '')}</dd>
				</div>
			{/each}
		</dl>
	{/if}
</div>
