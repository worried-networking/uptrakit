<script lang="ts">
	import { getSystemServicesSettings, updateSystemServicesSettings } from '$lib/api';
	import type { SystemServicesSettingsResponse } from '$lib/types';

	let {
		onSuccess,
		onError
	}: {
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let settings: SystemServicesSettingsResponse | null = $state(null);
	let loading: boolean = $state(false);
	let tokenInput: string = $state('');
	let showToken: boolean = $state(false);
	let saving: boolean = $state(false);
	let clearing: boolean = $state(false);
	let showClearConfirm: boolean = $state(false);

	$effect(() => {
		loadSettings();
	});

	async function loadSettings() {
		loading = true;
		try {
			settings = await getSystemServicesSettings();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to load system services settings');
		} finally {
			loading = false;
		}
	}

	async function saveToken() {
		const trimmed = tokenInput.trim();
		if (!trimmed) {
			onError('Enrollment token cannot be empty.');
			return;
		}
		saving = true;
		try {
			settings = await updateSystemServicesSettings({ enrollment_token: trimmed });
			tokenInput = '';
			showToken = false;
			onSuccess('System services enrollment token saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save enrollment token');
		} finally {
			saving = false;
		}
	}

	async function clearToken() {
		showClearConfirm = false;
		clearing = true;
		try {
			settings = await updateSystemServicesSettings({ enrollment_token: null });
			tokenInput = '';
			showToken = false;
			onSuccess('System services enrollment token cleared.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to clear enrollment token');
		} finally {
			clearing = false;
		}
	}
</script>

<div class="card mb-6 p-6">
	<h2 class="h3 mb-4">System Services</h2>
	<p class="mb-4 text-surface-600 dark:text-surface-400">
		Configure the shared enrollment token used by system services (e.g. dedicated tenant services) to authenticate with
		the controller.
	</p>

	{#if loading}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else if settings !== null}
		<div class="mb-4">
			<span class="label-text text-sm font-medium">Current Token</span>
			<p class="mt-1 text-sm text-surface-700 dark:text-surface-300">
				{settings.has_token ? 'A token is configured.' : '— not configured —'}
			</p>
		</div>

		<label class="label mb-4">
			<span>New Enrollment Token</span>
			<div class="flex gap-2">
				<input
					class="input font-mono flex-1"
					type={showToken ? 'text' : 'password'}
					placeholder="Paste token here"
					bind:value={tokenInput}
					autocomplete="off"
				/>
				<button
					class="btn btn-sm preset-tonal flex-shrink-0"
					type="button"
					onclick={() => (showToken = !showToken)}
					aria-label={showToken ? 'Hide token' : 'Show token'}
				>
					{showToken ? 'Hide' : 'Show'}
				</button>
			</div>
		</label>

		<div class="flex gap-2">
			<button class="btn preset-filled-primary-500" onclick={saveToken} disabled={saving || !tokenInput.trim()}>
				{saving ? 'Saving...' : 'Save Token'}
			</button>
			{#if settings.has_token}
				<button class="btn preset-tonal-error" onclick={() => (showClearConfirm = true)} disabled={clearing}>
					{clearing ? 'Clearing...' : 'Clear Token'}
				</button>
			{/if}
		</div>

		{#if showClearConfirm}
			<aside class="mt-4 rounded-lg border border-error-500 p-4">
				<p class="mb-3 text-sm font-medium">
					Are you sure you want to clear the system services enrollment token? System services using this token will no
					longer be able to enroll.
				</p>
				<div class="flex gap-2">
					<button class="btn btn-sm preset-filled-error-500" onclick={clearToken} disabled={clearing}>
						{clearing ? 'Clearing...' : 'Yes, Clear Token'}
					</button>
					<button class="btn btn-sm preset-tonal" onclick={() => (showClearConfirm = false)}>Cancel</button>
				</div>
			</aside>
		{/if}
	{/if}
</div>
