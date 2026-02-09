<script lang="ts">
	import { clearError, getErrorMessage, getSuccessMessage } from '$lib/notifications.svelte';

	const successMessage = $derived(getSuccessMessage());
	const errorMessage = $derived(getErrorMessage());
</script>

<div class="pointer-events-none fixed right-4 top-4 z-[70] flex w-full max-w-sm flex-col gap-2">
	{#if successMessage}
		<aside class="pointer-events-auto rounded-lg p-4 shadow-lg preset-filled-success-500">
			<p>{successMessage}</p>
		</aside>
	{/if}

	{#if errorMessage}
		<aside class="pointer-events-auto flex items-center justify-between gap-4 rounded-lg p-4 shadow-lg preset-filled-error-500">
			<p>{errorMessage}</p>
			<button class="btn btn-sm preset-filled" onclick={clearError}>Dismiss</button>
		</aside>
	{/if}
</div>
