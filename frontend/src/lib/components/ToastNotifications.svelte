<script lang="ts">
	import { clearError, getErrorMessage, getSuccessMessage } from '$lib/notifications.svelte';
	import { Callout, SectionCard, StatusBadge } from '$lib/components/ui';
	import type { SystemAlert } from '$lib/types';

	let { alerts, onDismiss }: { alerts: SystemAlert[]; onDismiss: (id: string) => void } = $props();

	const successMessage = $derived(getSuccessMessage());
	const errorMessage = $derived(getErrorMessage());

	function systemAlertTone(alert: SystemAlert): 'warning' | 'info' | 'danger' {
		if (alert.severity === 'critical' || alert.severity === 'error') return 'danger';
		if (alert.severity === 'warning') return 'warning';
		return 'info';
	}
</script>

<div
	class="pointer-events-none fixed bottom-[calc(4.5rem+env(safe-area-inset-bottom))] left-1/2 z-[70] flex w-[min(20rem,calc(100vw-1.5rem))] max-w-[calc(100vw-1.5rem)] -translate-x-1/2 flex-col gap-1.5 sm:bottom-auto sm:left-auto sm:right-4 sm:top-4 sm:w-[300px] sm:max-w-[calc(100vw-2rem)] sm:translate-x-0"
	aria-label="Notifications"
	data-ui="toast-notifications"
>
	{#if successMessage}
		<div class="pointer-events-auto" data-ui="toast-notification">
			<SectionCard>
				{#snippet actions()}
					<StatusBadge tone="success" label="Success" />
				{/snippet}
				<Callout tone="success" message={successMessage} />
			</SectionCard>
		</div>
	{/if}

	{#if errorMessage}
		<div class="pointer-events-auto" data-ui="toast-notification">
			<SectionCard>
				{#snippet actions()}
					<StatusBadge tone="danger" label="Error" />
					<button class="btn btn-sm preset-tonal-surface" onclick={clearError}>Dismiss</button>
				{/snippet}
				<Callout tone="danger" message={errorMessage} />
			</SectionCard>
		</div>
	{/if}

	{#each alerts as alert (alert.id)}
		<div class="pointer-events-auto" data-ui="toast-notification">
			<SectionCard>
				{#snippet actions()}
					<StatusBadge tone={systemAlertTone(alert)} label={alert.severity} />
				{/snippet}
				<div class="space-y-2">
					<Callout tone={systemAlertTone(alert)} title={alert.title} message={alert.message} />
					<div class="flex flex-wrap gap-2">
						{#if alert.action === 'renew_server_certificate'}
							<a href="/settings/global" class="btn btn-sm preset-tonal">Go to Global Settings</a>
						{/if}
						<button class="btn btn-sm preset-tonal-surface" onclick={() => onDismiss(alert.id)}>Dismiss</button>
					</div>
				</div>
			</SectionCard>
		</div>
	{/each}
</div>
