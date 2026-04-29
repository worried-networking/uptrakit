<script lang="ts">
	import { SvelteMap, SvelteSet } from 'svelte/reactivity';
	import type { SoftwareItemDetailResponse, SoftwareItemHostSummary, SoftwareItemResponse } from '$lib/types';
	import { formatVersion, isValidLogoUrl, resolveDisplayVersion } from '$lib/utils';
	import { ActionBadge, PillBadge, StatusBadge, TableFooterBar } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';
	import { Checkbox } from '$lib/components/forms';
	import UpdateAllButton from '$lib/components/UpdateAllButton.svelte';

	let {
		items,
		itemDetailsById,
		itemDetailLoadingIds,
		collapsedGroupIds,
		expandedOverflowGroupIds,
		batchSelectedIds,
		canManage,
		canTriggerUpdates,
		pluginTypeNames,
		totalItems,
		currentPage,
		totalPages,
		onToggleGroup,
		onToggleOverflow,
		onToggleBatch,
		onOpenMenu,
		onOpenUpdateModal,
		onPageChange,
		onToggleFeatured
	}: {
		items: SoftwareItemResponse[];
		itemDetailsById: SvelteMap<string, SoftwareItemDetailResponse>;
		itemDetailLoadingIds: SvelteSet<string>;
		collapsedGroupIds: SvelteSet<string>;
		expandedOverflowGroupIds: SvelteSet<string>;
		batchSelectedIds: SvelteSet<string>;
		canManage: boolean;
		canTriggerUpdates: boolean;
		pluginTypeNames: Map<string, string>;
		totalItems: number;
		currentPage: number;
		totalPages: number;
		onToggleGroup: (id: string) => void;
		onToggleOverflow: (id: string) => void;
		onToggleBatch: (id: string) => void;
		onOpenMenu: (id: string, button: HTMLElement) => void;
		onOpenUpdateModal: (item: SoftwareItemResponse) => void;
		onPageChange: (page: number) => void;
		onToggleFeatured: (item: SoftwareItemResponse) => void;
	} = $props();

	function detailHosts(item: SoftwareItemResponse): SoftwareItemDetailResponse['hosts'] {
		return itemDetailsById.get(item.id)?.hosts ?? [];
	}

	function visibleHosts(item: SoftwareItemResponse): SoftwareItemDetailResponse['hosts'] {
		const hosts = detailHosts(item);
		if (collapsedGroupIds.has(item.id)) return [];
		if (expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) return hosts;
		return hosts.slice(0, 3);
	}

	function hiddenHostCount(item: SoftwareItemResponse): number {
		const hosts = detailHosts(item);
		if (collapsedGroupIds.has(item.id) || expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) return 0;
		return hosts.length - 3;
	}

	function hiddenHostsSummary(item: SoftwareItemResponse): string {
		const hosts = detailHosts(item);
		if (collapsedGroupIds.has(item.id) || expandedOverflowGroupIds.has(item.id) || hosts.length <= 3) return '';
		const updateCount = hosts.slice(3).filter((h) => h.update_available && h.latest_version).length;
		return updateCount === 0 ? 'all up to date' : `${updateCount} with update${updateCount === 1 ? '' : 's'}`;
	}

	function updateableHostCount(item: SoftwareItemResponse): number | null {
		const hosts = detailHosts(item);
		if (hosts.length > 0) return hosts.filter((h) => h.update_available && h.latest_version).length;
		return null;
	}

	function hasAnyUpdateableHosts(item: SoftwareItemResponse): boolean {
		const c = updateableHostCount(item);
		return c === null ? item.update_available : c > 0;
	}

	function softwareUpdateLabel(item: SoftwareItemResponse): string {
		const c = updateableHostCount(item);
		return c === null ? 'loading updates' : c === 0 ? 'up to date' : `${c} update${c === 1 ? '' : 's'}`;
	}

	function primaryPluginLabel(item: SoftwareItemResponse, host?: SoftwareItemHostSummary): string {
		const plugin = host?.plugins[0];
		if (plugin?.plugin_config_name) return plugin.plugin_config_name;
		if (plugin?.plugin_type) return pluginTypeNames.get(plugin.plugin_type) ?? plugin.plugin_type;
		const itemPlugin = item.plugins[0];
		return itemPlugin ? (pluginTypeNames.get(itemPlugin) ?? itemPlugin) : 'Unknown';
	}

	function hostDisplayName(host: SoftwareItemHostSummary): string {
		return host.friendly_name || host.hostname;
	}

	function isSingleHostItem(item: SoftwareItemResponse): boolean {
		const hosts = detailHosts(item);
		return hosts.length > 0 ? hosts.length === 1 : item.host_count === 1;
	}

	function singleHost(item: SoftwareItemResponse): SoftwareItemHostSummary | null {
		const hosts = detailHosts(item);
		return hosts.length === 1 ? hosts[0] : null;
	}

	function versionLabel(
		version: string | null | undefined,
		displayVersion?: string | null | undefined,
		fallback = '—'
	): string {
		if (!version) return fallback;
		return formatVersion(resolveDisplayVersion(version, displayVersion ?? undefined));
	}

	function versionTitle(version: string | null | undefined, displayVersion?: string | null | undefined): string {
		return resolveDisplayVersion(version, displayVersion ?? undefined) ?? '—';
	}

	function groupIsOpen(itemId: string): boolean {
		return !collapsedGroupIds.has(itemId);
	}
</script>

<!-- Desktop layout: hidden on mobile (< 640px) -->
<div class="max-sm:hidden" data-ui="software-group-list" role="list" aria-label="Tracked software">
	{#each items as item (item.id)}
		{@const compactSingleHost = singleHost(item)}
		{@const isCompactSingleHost = isSingleHostItem(item)}
		<div
			class="border-b border-[var(--border-subtle)] last:border-b-0"
			data-testid={'software-group-' + item.id}
			role="listitem"
		>
			<div
				class="grid items-center gap-x-2 bg-[var(--bg-raised)] px-4 py-2.5 {canManage
					? 'grid-cols-[24px_minmax(0,1fr)_40px]'
					: 'grid-cols-[minmax(0,1fr)]'}"
				data-testid={'software-group-header-' + item.id}
			>
				{#if canManage}
					<div>
						<Checkbox
							id={'software-row-' + item.id}
							checked={batchSelectedIds.has(item.id)}
							onchange={() => onToggleBatch(item.id)}
							aria-label={'Select ' + item.name}
						/>
					</div>
				{/if}
				<div class="grid grid-cols-[1fr_140px_88px] items-center gap-x-2" data-ui="software-group-grid">
					<div class="min-w-0">
						<div class="flex items-center gap-2">
							{#if canManage}
								<button
									class="cursor-pointer text-section-title leading-none transition-[background,border-color,color] duration-fast hover:text-[var(--accent-bright)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
									class:text-[var(--color-warning)]={item.featured}
									class:star-unfeatured={!item.featured}
									title={item.featured ? 'Unfeature' : 'Feature'}
									onclick={(e) => {
										e.stopPropagation();
										onToggleFeatured(item);
									}}
									aria-label={(item.featured ? 'Unfeature ' : 'Feature ') + item.name}
								>
									{item.featured ? '★' : '☆'}
								</button>
							{:else}
								<span
									class={item.featured
										? 'text-section-title leading-none text-[var(--color-warning)]'
										: 'star-unfeatured text-section-title leading-none'}>{item.featured ? '★' : '☆'}</span
								>
							{/if}
							{#if isValidLogoUrl(item.icon_url)}
								<img
									src={item.icon_url}
									alt=""
									class="h-5 w-5 rounded-panel object-contain"
									referrerpolicy="no-referrer"
								/>
							{/if}
							<a
								href={'/software/' + item.id}
								class="truncate text-sm font-semibold text-[var(--text-primary)] hover:underline"
							>
								{item.name}
							</a>
						</div>
						{#if isCompactSingleHost && compactSingleHost}
							<div class="mt-0.5 flex items-center gap-2">
								<p class="truncate text-nav-item text-[var(--text-secondary)]">
									{hostDisplayName(compactSingleHost)}
								</p>
								<PillBadge label={primaryPluginLabel(item, compactSingleHost)} />
							</div>
						{:else}
							<div class="mt-0.5 flex items-center gap-1">
								<button
									type="button"
									class="expand-pill min-h-badge"
									aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
									aria-expanded={groupIsOpen(item.id)}
									aria-controls={'software-group-body-' + item.id}
									onclick={() => onToggleGroup(item.id)}
								>
									<span
										class={groupIsOpen(item.id)
											? 'shrink-0 text-subsection-title leading-none'
											: 'shrink-0 text-table-header leading-none'}
										aria-hidden="true">{groupIsOpen(item.id) ? '▼' : '▶'}</span
									>
									<span>{item.host_count} host{item.host_count === 1 ? '' : 's'}</span>
								</button>
								<span class="text-nav-item text-[var(--text-secondary)]">· {softwareUpdateLabel(item)}</span>
							</div>
						{/if}
					</div>
					{#if isCompactSingleHost && compactSingleHost}
						<div class="text-right">
							<p
								class="font-mono text-nav-item text-[var(--text-secondary)]"
								title={versionTitle(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
							>
								{versionLabel(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
							</p>
							{#if compactSingleHost.update_available && compactSingleHost.latest_version}
								<p
									class="font-mono text-button text-[var(--accent-bright)]"
									title={versionTitle(
										compactSingleHost.latest_version,
										(compactSingleHost.latest_release_metadata?.display_version as string | null | undefined) ??
											undefined
									)}
								>
									↑ {versionLabel(
										compactSingleHost.latest_version,
										(compactSingleHost.latest_release_metadata?.display_version as string | null | undefined) ??
											undefined
									)}
								</p>
							{/if}
						</div>
					{:else}
						<div aria-hidden="true"></div>
					{/if}
					<div class="flex justify-end">
						{#if canTriggerUpdates}
							{#if isCompactSingleHost}
								<ActionBadge
									variant="navigation"
									tone="accent"
									idleLabel="Update"
									hoverLabel="Update"
									disabled={!(compactSingleHost?.update_available && compactSingleHost?.latest_version)}
									onclick={() => onOpenUpdateModal(item)}
								/>
							{:else}
								<UpdateAllButton
									state={hasAnyUpdateableHosts(item) ? 'idle' : 'dim'}
									ariaLabel={hasAnyUpdateableHosts(item) ? undefined : 'No updates available'}
									onclick={() => onOpenUpdateModal(item)}
								/>
							{/if}
						{:else if isCompactSingleHost && compactSingleHost?.update_available}
							<StatusBadge tone="info" label="Update avail" />
						{:else if hasAnyUpdateableHosts(item)}
							{@const groupUpdateCount = updateableHostCount(item)}
							<StatusBadge
								tone="info"
								label={groupUpdateCount === null
									? 'Updates avail'
									: `${groupUpdateCount} update${groupUpdateCount === 1 ? '' : 's'}`}
							/>
						{:else}
							<StatusBadge tone="success" label="Up to date" />
						{/if}
					</div>
				</div>
				{#if canManage}
					<div class="actions-menu flex justify-end">
						<Button
							variant="ghost"
							size="sm"
							ariaLabel={'Actions for ' + item.name}
							onclick={(e) => {
								e.stopPropagation();
								onOpenMenu(item.id, e.currentTarget);
							}}>&#8943;</Button
						>
					</div>
				{/if}
			</div>
			{#if !isCompactSingleHost && itemDetailLoadingIds.has(item.id)}
				<div
					class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] px-4 py-2.5 {canManage
						? 'grid-cols-[24px_minmax(0,1fr)_40px]'
						: 'grid-cols-[minmax(0,1fr)]'}"
					id={'software-group-body-' + item.id}
				>
					{#if canManage}
						<span aria-hidden="true"></span>
					{/if}
					<div class="grid grid-cols-[8px_1fr_140px_88px] items-center gap-x-3">
						<div class="col-[1/5] text-sm text-[var(--text-secondary)]">Loading hosts...</div>
					</div>
					{#if canManage}
						<span aria-hidden="true"></span>
					{/if}
				</div>
			{:else if !isCompactSingleHost && detailHosts(item).length > 0}
				<div id={'software-group-body-' + item.id}>
					{#each visibleHosts(item) as host (host.id)}
						<div
							class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2.5 transition-[background,border-color,color] duration-fast hover:bg-[var(--bg-raised)] {canManage
								? 'grid-cols-[24px_minmax(0,1fr)_40px]'
								: 'grid-cols-[minmax(0,1fr)]'}"
							data-testid={'software-host-row-' + host.id}
						>
							{#if canManage}
								<span aria-hidden="true"></span>
							{/if}
							<div class="grid grid-cols-[1fr_140px_88px] items-center gap-x-2" data-ui="software-host-grid">
								<div class="min-w-0 pl-[18px]">
									<div class="flex min-w-0 items-center gap-2">
										<span class="shrink-0 text-table-header text-[var(--text-secondary)]" aria-hidden="true">·</span>
										<p class="truncate text-sm text-[var(--text-primary)]">{hostDisplayName(host)}</p>
										<PillBadge label={primaryPluginLabel(item, host)} />
									</div>
									{#if hostDisplayName(host) !== host.hostname}
										<p class="mt-1 truncate text-nav-item text-[var(--text-secondary)]">{host.hostname}</p>
									{/if}
								</div>
								<div class="text-right">
									<p
										class="font-mono text-nav-item text-[var(--text-secondary)]"
										title={versionTitle(host.installed_version, host.installed_display_version)}
									>
										{versionLabel(host.installed_version, host.installed_display_version)}
									</p>
									{#if host.update_available && host.latest_version}
										<p
											class="font-mono text-button text-[var(--accent-bright)]"
											title={versionTitle(
												host.latest_version,
												(host.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
											)}
										>
											↑ {versionLabel(
												host.latest_version,
												(host.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
											)}
										</p>
									{/if}
								</div>
								<div class="flex justify-end">
									{#if host.update_available && canTriggerUpdates}
										<ActionBadge
											variant="navigation"
											tone="accent"
											idleLabel="Update"
											hoverLabel="Update"
											onclick={() => onOpenUpdateModal(item)}
										/>
									{:else if host.update_available}
										<StatusBadge tone="info" label="Update avail" />
									{:else}
										<StatusBadge tone="success" label="Up to date" />
									{/if}
								</div>
							</div>
							{#if canManage}
								<span aria-hidden="true"></span>
							{/if}
						</div>
					{/each}
					{#if hiddenHostCount(item) > 0}
						<div
							class="grid items-center gap-x-2 border-t border-[var(--border-subtle)] bg-transparent px-4 py-2.5 {canManage
								? 'grid-cols-[24px_minmax(0,1fr)_40px]'
								: 'grid-cols-[minmax(0,1fr)]'}"
						>
							{#if canManage}
								<span aria-hidden="true"></span>
							{/if}
							<div class="grid grid-cols-[8px_1fr_140px_88px] items-center gap-x-3">
								<span aria-hidden="true"></span>
								<div>
									<button
										type="button"
										class="pl-[49px] text-nav-item text-[var(--text-secondary)] transition-[background,border-color,color] duration-fast hover:text-[var(--text-primary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
										onclick={() => onToggleOverflow(item.id)}
									>
										▸ {hiddenHostCount(item)} more — {hiddenHostsSummary(item)}
									</button>
								</div>
								<span aria-hidden="true"></span>
								<span aria-hidden="true"></span>
							</div>
							{#if canManage}
								<span aria-hidden="true"></span>
							{/if}
						</div>
					{/if}
				</div>
			{/if}
		</div>
	{/each}
	<TableFooterBar total={totalItems} {currentPage} {totalPages} {onPageChange} />
</div>

<!-- Mobile card layout: visible only on mobile (< 640px) -->
<div
	class="sm:hidden divide-y divide-[var(--border-subtle)]"
	data-ui="software-group-list-mobile"
	role="list"
	aria-label="Tracked software"
>
	{#each items as item (item.id)}
		{@const compactSingleHost = singleHost(item)}
		{@const isCompactSingleHost = isSingleHostItem(item)}
		<div class="px-4 py-3" data-testid={'software-group-mobile-' + item.id} role="listitem">
			<!-- Card header: checkbox + star + icon + name + actions button -->
			<div class="flex min-w-0 items-center gap-2">
				{#if canManage}
					<Checkbox
						id={'software-row-mobile-' + item.id}
						checked={batchSelectedIds.has(item.id)}
						onchange={() => onToggleBatch(item.id)}
						aria-label={'Select ' + item.name}
					/>
				{/if}
				<!-- Star is display-only on mobile; use the ⋯ actions menu to Feature/Unfeature. -->
				<span
					class={item.featured
						? 'shrink-0 text-section-title leading-none text-[var(--color-warning)]'
						: 'shrink-0 star-unfeatured text-section-title leading-none'}
				>
					{item.featured ? '★' : '☆'}
				</span>
				{#if isValidLogoUrl(item.icon_url)}
					<img
						src={item.icon_url}
						alt=""
						class="h-4 w-4 shrink-0 rounded-panel object-contain"
						referrerpolicy="no-referrer"
					/>
				{/if}
				<a
					href={'/software/' + item.id}
					class="min-w-0 truncate text-sm font-semibold text-[var(--text-primary)] hover:underline"
				>
					{item.name}
				</a>
				{#if canManage}
					<Button
						variant="ghost"
						size="sm"
						class="ml-auto shrink-0"
						ariaLabel={'Actions for ' + item.name}
						onclick={(e) => {
							e.stopPropagation();
							onOpenMenu(item.id, e.currentTarget);
						}}>&#8943;</Button
					>
				{/if}
			</div>

			{#if isCompactSingleHost && compactSingleHost}
				<!-- Compact single-host: hostname + plugin badge inline -->
				<div class="mt-0.5 flex items-center gap-2">
					<p class="truncate text-nav-item text-[var(--text-secondary)]">{hostDisplayName(compactSingleHost)}</p>
					<PillBadge label={primaryPluginLabel(item, compactSingleHost)} />
				</div>
				<!-- Version + action row -->
				<div class="mt-1.5 flex items-center justify-between gap-2">
					<div class="min-w-0">
						<p
							class="truncate font-mono text-nav-item text-[var(--text-secondary)]"
							title={versionTitle(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
						>
							{versionLabel(compactSingleHost.installed_version, compactSingleHost.installed_display_version)}
						</p>
						{#if compactSingleHost.update_available && compactSingleHost.latest_version}
							<p class="truncate font-mono text-button text-[var(--accent-bright)]">
								↑ {versionLabel(
									compactSingleHost.latest_version,
									(compactSingleHost.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
								)}
							</p>
						{/if}
					</div>
					<div class="shrink-0">
						{#if canTriggerUpdates}
							<ActionBadge
								variant="navigation"
								tone="accent"
								idleLabel="Update"
								hoverLabel="Update"
								disabled={!(compactSingleHost.update_available && compactSingleHost.latest_version)}
								onclick={() => onOpenUpdateModal(item)}
							/>
						{:else if compactSingleHost.update_available}
							<StatusBadge tone="info" label="Update avail" />
						{:else}
							<StatusBadge tone="success" label="Up to date" />
						{/if}
					</div>
				</div>
			{:else}
				<!-- Multi-host: expand pill + update summary -->
				<div class="mt-0.5 flex items-center gap-2">
					<button
						type="button"
						class="expand-pill min-h-badge"
						aria-label={groupIsOpen(item.id) ? 'Collapse ' + item.name : 'Expand ' + item.name}
						aria-expanded={groupIsOpen(item.id)}
						aria-controls={'software-group-mobile-body-' + item.id}
						onclick={() => onToggleGroup(item.id)}
					>
						<span
							class={groupIsOpen(item.id)
								? 'shrink-0 text-subsection-title leading-none'
								: 'shrink-0 text-table-header leading-none'}
							aria-hidden="true">{groupIsOpen(item.id) ? '▼' : '▶'}</span
						>
						<span>{item.host_count} host{item.host_count === 1 ? '' : 's'}</span>
					</button>
					<span class="text-nav-item text-[var(--text-secondary)]">· {softwareUpdateLabel(item)}</span>
				</div>

				<!-- Host sub-cards (expanded) -->
				{#if itemDetailLoadingIds.has(item.id) && groupIsOpen(item.id)}
					<p class="mt-1 pl-3 text-sm text-[var(--text-secondary)]">Loading hosts...</p>
				{:else if groupIsOpen(item.id) && detailHosts(item).length > 0}
					<div
						class="mt-2 space-y-2 border-l-2 border-[var(--border-subtle)] pl-3"
						id={'software-group-mobile-body-' + item.id}
					>
						{#each visibleHosts(item) as host (host.id)}
							<div class="flex items-start justify-between gap-2" data-testid={'software-host-mobile-row-' + host.id}>
								<div class="min-w-0">
									<div class="flex min-w-0 items-center gap-2">
										<span class="shrink-0 text-table-header text-[var(--text-secondary)]" aria-hidden="true">·</span>
										<p class="truncate text-sm text-[var(--text-primary)]">{hostDisplayName(host)}</p>
										<PillBadge label={primaryPluginLabel(item, host)} />
									</div>
									{#if hostDisplayName(host) !== host.hostname}
										<p class="mt-0.5 truncate text-nav-item text-[var(--text-secondary)]">{host.hostname}</p>
									{/if}
									<p
										class="font-mono text-nav-item text-[var(--text-secondary)]"
										title={versionTitle(host.installed_version, host.installed_display_version)}
									>
										{versionLabel(host.installed_version, host.installed_display_version)}
									</p>
									{#if host.update_available && host.latest_version}
										<p class="font-mono text-button text-[var(--accent-bright)]">
											↑ {versionLabel(
												host.latest_version,
												(host.latest_release_metadata?.display_version as string | null | undefined) ?? undefined
											)}
										</p>
									{/if}
								</div>
								<div class="shrink-0">
									{#if host.update_available && canTriggerUpdates}
										<ActionBadge
											variant="navigation"
											tone="accent"
											idleLabel="Update"
											hoverLabel="Update"
											onclick={() => onOpenUpdateModal(item)}
										/>
									{:else if host.update_available}
										<StatusBadge tone="info" label="Update avail" />
									{:else}
										<StatusBadge tone="success" label="Up to date" />
									{/if}
								</div>
							</div>
						{/each}
						{#if hiddenHostCount(item) > 0}
							<button
								type="button"
								class="text-nav-item text-[var(--text-secondary)] transition-[color] duration-fast hover:text-[var(--text-primary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
								onclick={() => onToggleOverflow(item.id)}
							>
								▸ {hiddenHostCount(item)} more — {hiddenHostsSummary(item)}
							</button>
						{/if}
					</div>
				{/if}
			{/if}
		</div>
	{/each}
	<TableFooterBar total={totalItems} {currentPage} {totalPages} {onPageChange} />
</div>

<style>
	.expand-pill {
		display: inline-flex;
		min-height: 14px;
		align-items: center;
		overflow: hidden;
		border-radius: var(--radius-badge);
		border: 1px solid rgba(var(--accent-rgb), 0.22);
		background: rgba(var(--accent-rgb), 0.08);
		padding: 0 5px;
		font-size: var(--text-button);
		font-weight: 600;
		text-transform: none;
		gap: 3px;
		color: var(--accent);
		transition:
			background 0.12s,
			border-color 0.12s,
			color 0.12s;
	}
	.expand-pill:hover {
		background: rgba(var(--accent-rgb), 0.18);
		border-color: rgba(var(--accent-rgb), 0.42);
		color: var(--accent-bright);
	}
	.expand-pill:focus-visible {
		outline: none;
		box-shadow: 0 0 0 3px rgba(var(--accent-rgb), 0.25);
	}
	.star-unfeatured {
		color: var(--text-secondary);
	}
</style>
