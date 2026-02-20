<script lang="ts">
	import { clearError, getErrorMessage, getSuccessMessage } from '$lib/notifications.svelte';
	import type { SystemAlert } from '$lib/types';

	let { alerts, onDismiss }: { alerts: SystemAlert[]; onDismiss: (id: string) => void } = $props();

	const successMessage = $derived(getSuccessMessage());
	const errorMessage = $derived(getErrorMessage());
</script>

<div class="pointer-events-none fixed right-4 top-4 z-[70] flex w-full max-w-sm flex-col gap-2" aria-label="Notifications">
	{#if successMessage}
		<aside role="status" aria-live="polite" aria-atomic="true" class="pointer-events-auto rounded-lg p-4 shadow-lg preset-filled-success-500">
			<p>{successMessage}</p>
		</aside>
	{/if}

	{#if errorMessage}
		<aside role="alert" aria-live="assertive" aria-atomic="true" class="pointer-events-auto flex items-center justify-between gap-4 rounded-lg p-4 shadow-lg preset-filled-error-500">
			<p>{errorMessage}</p>
			<button class="btn btn-sm preset-filled" onclick={clearError}>Dismiss</button>
		</aside>
	{/if}

	{#each alerts as alert (alert.id)}
		<aside role="status" aria-live="polite" aria-atomic="true" class="pointer-events-auto flex items-center justify-between gap-4 rounded-lg p-4 shadow-lg {alert.severity === 'warning' ? 'preset-filled-warning-500' : 'preset-filled-surface-400-600'}">
			<div>
				<h3 class="font-bold">{alert.title}</h3>
				<p>{alert.message}</p>
			</div>
			<div class="flex gap-2">
				{#if alert.action === 'renew_server_certificate'}
					<a href="/settings/global" class="btn btn-sm preset-filled">Go to Global Settings</a>
				{/if}
				<button class="btn btn-sm preset-filled" onclick={() => onDismiss(alert.id)}>Dismiss</button>
			</div>
		</aside>
	{/each}
</div>
