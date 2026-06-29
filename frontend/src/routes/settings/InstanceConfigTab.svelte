<script lang="ts">
	import { getUser } from '$lib/auth.svelte';
	import { getConfigState, clearCoordinatorDegraded } from '$lib/api';
	import { Permission, hasAnyPermission } from '$lib/types';
	import type { ConfigStateResponse } from '$lib/api';
	import { showSuccess, showError } from '$lib/notifications.svelte';
	import { Callout, SectionCard } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import { formatDate } from '$lib/utils';

	let configState: ConfigStateResponse | null = $state(null);
	let loading: boolean = $state(false);
	let error: string | null = $state(null);
	let clearing: boolean = $state(false);

	const canManage = $derived(hasAnyPermission(getUser(), Permission.ManageInstanceConfigState));

	async function load() {
		loading = true;
		error = null;
		try {
			const { data: cs } = await getConfigState();
			configState = cs;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load config state';
		} finally {
			loading = false;
		}
	}

	async function clearDegraded() {
		clearing = true;
		try {
			const { data: cs2 } = await clearCoordinatorDegraded();
			configState = cs2;
			showSuccess('Degraded state cleared');
		} catch (e) {
			showError(e instanceof Error ? e.message : 'Failed to clear degraded state');
		} finally {
			clearing = false;
		}
	}

	$effect(() => {
		load();
	});
</script>

<div class="space-y-4">
	{#if loading && !configState}
		<SectionCard title="Instance Configuration">
			<p class="py-4 text-center text-sm text-[var(--text-secondary)]">Loading...</p>
		</SectionCard>
	{:else if error}
		<Callout tone="danger" title="Failed to load config state" message={error}>
			<div class="mt-2">
				<Button variant="primary" size="sm" onclick={load}>Retry</Button>
			</div>
		</Callout>
	{:else if configState}
		<!-- Coordinator state banner -->
		{#if configState.coordinator_state === 'degraded' && configState.degraded}
			<Callout
				tone="danger"
				title="Coordinator degraded"
				message="{configState.degraded.reason} (since {formatDate(configState.degraded.since)})"
			>
				{#if canManage}
					<div class="mt-2">
						<!-- No "Reload Now" button — spec §15.5 explicitly forbids it; reload is triggered
							 only via SIGHUP, file-watch, or the DB reconciler, not from the UI. -->
						<Button variant="primary" size="sm" loading={clearing} onclick={clearDegraded}>Clear Degraded</Button>
					</div>
				{/if}
			</Callout>
		{/if}

		<!-- File state -->
		<SectionCard title="Config File">
			<dl class="grid grid-cols-1 gap-2 text-sm sm:grid-cols-2">
				<div>
					<dt class="text-[var(--text-secondary)]">Path</dt>
					<dd class="font-mono text-[var(--text-primary)]">{configState.file.path}</dd>
				</div>
				<div>
					<dt class="text-[var(--text-secondary)]">Digest</dt>
					<dd class="font-mono text-[var(--text-primary)]">{configState.file.digest}</dd>
				</div>
				<div>
					<dt class="text-[var(--text-secondary)]">Loaded at</dt>
					<dd class="text-[var(--text-primary)]">{formatDate(configState.file.loaded_at)}</dd>
				</div>
				{#if configState.file.pending_digest}
					<div>
						<dt class="text-[var(--text-secondary)]">Pending change</dt>
						<dd class="font-mono text-[var(--color-warning)]">{configState.file.pending_digest}</dd>
					</div>
				{/if}
			</dl>
		</SectionCard>

		<!-- Last reload -->
		{#if configState.last_reload}
			<SectionCard title="Last Reload">
				<dl class="grid grid-cols-1 gap-2 text-sm sm:grid-cols-2">
					<div>
						<dt class="text-[var(--text-secondary)]">Completed at</dt>
						<dd class="text-[var(--text-primary)]">{formatDate(configState.last_reload.completed_at)}</dd>
					</div>
					<div>
						<dt class="text-[var(--text-secondary)]">Changed sections</dt>
						<dd class="text-[var(--text-primary)]">{configState.last_reload.sections.join(', ') || '—'}</dd>
					</div>
				</dl>
				{#if Object.keys(configState.last_reload.per_subsystem_ms).length > 0}
					<details class="mt-3">
						<summary class="cursor-pointer text-xs text-[var(--text-secondary)]">Per-subsystem timing</summary>
						<dl class="mt-2 grid grid-cols-2 gap-1 text-xs">
							{#each Object.entries(configState.last_reload.per_subsystem_ms) as [name, ms] (name)}
								<div class="flex gap-2">
									<dt class="text-[var(--text-secondary)]">{name}</dt>
									<dd class="text-[var(--text-primary)]">{ms} ms</dd>
								</div>
							{/each}
						</dl>
					</details>
				{/if}
			</SectionCard>
		{/if}

		<!-- Active config sections -->
		<SectionCard title="Active Configuration Sections">
			<pre
				class="overflow-auto rounded-card bg-[var(--bg-raised)] p-4 text-xs text-[var(--text-primary)]">{JSON.stringify(
					configState.sections,
					null,
					2
				)}</pre>
		</SectionCard>

		<!-- Recent events -->
		{#if configState.recent_events.length > 0}
			<SectionCard title="Recent Reload Events">
				<div class="space-y-2">
					{#each [...configState.recent_events].reverse() as event, i (i)}
						<div
							class="rounded border border-[var(--border-subtle)] bg-[var(--bg-raised)] p-3 text-xs font-mono text-[var(--text-primary)]"
						>
							<pre class="overflow-auto">{JSON.stringify(event, null, 2)}</pre>
						</div>
					{/each}
				</div>
			</SectionCard>
		{/if}

		<!-- Refresh button -->
		<div class="flex justify-end">
			<Button variant="secondary" size="sm" {loading} onclick={load}>Refresh</Button>
		</div>
	{/if}
</div>
