import type { ExtensionResponse } from './types';
import { listExtensions } from './api';
import { SvelteMap } from 'svelte/reactivity';

let extensions: ExtensionResponse[] = $state([]);
let loaded: boolean = $state(false);

export function getExtensions(): ExtensionResponse[] {
	return extensions;
}

export function getExtensionsLoaded(): boolean {
	return loaded;
}

export async function loadExtensions(): Promise<void> {
	try {
		extensions = await listExtensions();
		loaded = true;
	} catch (e) {
		console.error('Failed to load extensions:', e);
		extensions = [];
		loaded = true;
	}
}

export function clearExtensions(): void {
	extensions = [];
	loaded = false;
}

export function getPageExtensions(): ExtensionResponse[] {
	return extensions.filter((e) => e.placement.type === 'page');
}

export function getPanelExtensions(targetPage: string): ExtensionResponse[] {
	return extensions.filter((e) => e.placement.type === 'panel' && e.placement.target_page === targetPage);
}

export function getTabExtensions(targetPage: string): ExtensionResponse[] {
	return extensions.filter(
		(e) =>
			e.placement.type === 'panel' &&
			(e.placement as { type: 'panel'; target_page: string; position: { type: string } }).target_page === targetPage &&
			(e.placement as { type: 'panel'; target_page: string; position: { type: string } }).position.type === 'tab'
	);
}

export function getContextMenuExtensions(targetEntity: string): ExtensionResponse[] {
	return extensions.filter(
		(e) => e.placement.type === 'context_menu_group' && e.placement.target_entity === targetEntity
	);
}

export function getTableExtensions(targetTable: string): ExtensionResponse[] {
	return extensions.filter((e) => e.placement.type === 'table_columns' && e.placement.target_table === targetTable);
}

export interface TabGroupResult {
	/** Extensions without a tab_group — each gets its own tab. */
	ungrouped: ExtensionResponse[];
	/** Extensions sharing the same tab_group — one tab per group, multiple sections inside. */
	groups: SvelteMap<string, ExtensionResponse[]>;
}

/**
 * Returns tab extensions split into ungrouped (one tab each) and grouped
 * (one tab per tab_group value, multiple extensions rendered as sections).
 */
export function getGroupedTabExtensions(targetPage: string): TabGroupResult {
	const tabs = getTabExtensions(targetPage);
	const ungrouped: ExtensionResponse[] = [];
	const groups = new SvelteMap<string, ExtensionResponse[]>();

	for (const ext of tabs) {
		const group =
			ext.placement.type === 'panel' ? (ext.placement as { type: 'panel'; tab_group?: string }).tab_group : undefined;
		if (group) {
			const existing = groups.get(group);
			if (existing) {
				existing.push(ext);
			} else {
				groups.set(group, [ext]);
			}
		} else {
			ungrouped.push(ext);
		}
	}

	return { ungrouped, groups };
}

/**
 * Returns panel extensions positioned "below" the target page content.
 */
export function getBelowExtensions(targetPage: string): ExtensionResponse[] {
	return extensions.filter(
		(e) =>
			e.placement.type === 'panel' &&
			(e.placement as { type: 'panel'; target_page: string; position: { type: string } }).target_page === targetPage &&
			(e.placement as { type: 'panel'; target_page: string; position: { type: string } }).position.type === 'below'
	);
}
